// 网页宿主: 实现 HOST_PROTOCOL.md 里的全部调用。文件字节只存在于浏览器侧 (File 对象), 从不进入 NSH 的 VM。
// 无头测试 (nsh test) 会在没有 window 的上下文里加载这个模块, 所以所有 DOM 访问都在 hasDom 之后。

const hasDom = typeof window !== "undefined" && typeof document !== "undefined";
const IMAGE_EXT = /\.(png|jpe?g|webp|bmp|gif|tiff?|heic|heif)$/i;
const MAX_PICK = 500;

/** 宿主持有的文件: id -> File。上传成功或 files.release 后释放。 */
const files = new Map();
let seq = 0;

const isImage = (file) => file.type.startsWith("image/") || IMAGE_EXT.test(file.name);

/** `dated` 为假时不带修改时间: PDF 渲染出来的页是刚生成的, `lastModified` 只是渲染的那一刻, 不是用户的文件日期。 */
function register(list, { dated = true } = {}) {
  return list.map((file) => {
    const id = "w" + ++seq + "-" + Math.random().toString(36).slice(2, 8);
    files.set(id, file);
    const ref = { id, name: file.name, size: file.size, type: file.type };
    if (dated && file.lastModified > 0) ref.modifiedAt = file.lastModified;
    return ref;
  });
}

/** 页序靠名字里的数字: page2 排在 page10 前 (与 App 的文件夹导入一致)。 */
const naturalCompare = (a, b) => a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" });

/** 递归读取目录条目里的文件, 带上相对路径。dataTransfer 的条目必须在 drop 事件同步阶段取出, 之后才可以异步遍历。 */
async function walk(entry, out) {
  if (entry.isFile) {
    const file = await new Promise((resolve, reject) => entry.file(resolve, reject));
    out.push({ path: entry.fullPath, file });
    return;
  }
  if (!entry.isDirectory) return;
  const reader = entry.createReader();
  for (;;) {
    const batch = await new Promise((resolve, reject) => reader.readEntries(resolve, reject));
    if (batch.length === 0) break;
    for (const child of batch) await walk(child, out);
  }
}

async function filesFromTransfer(items, plainFiles) {
  const entries = [];
  for (const item of items) {
    const entry = item.kind === "file" && item.webkitGetAsEntry ? item.webkitGetAsEntry() : null;
    if (entry) entries.push(entry);
  }
  if (entries.length === 0) return [...plainFiles];
  const found = [];
  for (const entry of entries) await walk(entry, found);
  return found.sort((a, b) => naturalCompare(a.path, b.path)).map((item) => item.file);
}

function chooseFiles(accept, multiple, directory = false) {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.multiple = multiple;
    input.accept = accept;
    input.webkitdirectory = directory;
    input.style.display = "none";
    const finish = (list) => {
      input.remove();
      resolve(list);
    };
    input.addEventListener("change", () => finish([...input.files]));
    input.addEventListener("cancel", () => finish([]));
    document.body.appendChild(input);
    input.click();
  });
}

const KIND_BY_STATUS = new Map([
  [401, "unauthorized"],
  [429, "queue-full"],
  [503, "engine-stopped"],
  [400, "rejected"],
  [413, "rejected"],
  [415, "rejected"],
  [422, "rejected"],
]);

async function upload({ fileId, url, token, overrides }) {
  const file = files.get(fileId);
  if (!file) return { ok: false, kind: "rejected", message: "文件已不可用" };
  const form = new FormData();
  form.append("image", file, file.name);
  form.append("overrides", JSON.stringify(overrides ?? {}));
  let response;
  try {
    response = await fetch(url, { method: "POST", headers: { Authorization: "Bearer " + token }, body: form });
  } catch (error) {
    return { ok: false, kind: "network", message: String(error && error.message ? error.message : error) };
  }
  const body = await response.json().catch(() => null);
  if (response.ok && body && typeof body.job_id === "string") {
    files.delete(fileId);
    return { ok: true, jobId: body.job_id };
  }
  const kind = KIND_BY_STATUS.get(response.status) ?? (response.status >= 500 ? "network" : "rejected");
  const message = (body && typeof body.error === "string" && body.error) || "HTTP " + response.status;
  return { ok: false, kind, message };
}

const EXTENSIONS = { "image/png": "png", "image/jpeg": "jpg", "image/webp": "webp", "text/html": "html" };

/** 下载整份文件再让浏览器保存: 要从应答的 Content-Type 取扩展名, <a download> 自己不会加。字节只在页面里, 不进 NSH 的 VM。 */
async function saveArtifact({ url, filename }) {
  const response = await fetch(url);
  if (!response.ok) throw new Error("HTTP " + response.status);
  const blob = await response.blob();
  const extension = EXTENSIONS[blob.type.split(";")[0]] ?? "bin";
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = (filename || "translated") + "." + extension;
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(link.href), 10_000);
  return null;
}

// ---- PDF: pdf.js 覆盖层与页图渲染 (契约见 HOST_PROTOCOL.md 的「PDF 阅读器」) ----

/**
 * 宿主持有的 PDF: id -> { id, file, url, task, doc, users, idle }。url 是 object URL; task 是 pdf.js 的加载任务, doc 是它的文档 (Promise),
 * 第一次需要时才打开。打开的文档连同它的 worker 和已读入的数据可以占到几百 MB, 所以只在有人用着、或覆盖层正显示它时留着。
 */
const pdfs = new Map();
/** 覆盖层 canvas 的长边上限 (设备像素): 大屏高 DPR 下也不画出超大的缓冲。 */
const OVERLAY_MAX_EDGE = 3000;
/** 没人用的文档放这么久之后释放; 回到阅读器时再打开 (读一次文件头尾, 不到一秒)。 */
const DOC_IDLE_MS = 20_000;
const isPdf = (file) => file.type === "application/pdf" || /\.pdf$/i.test(file.name);

/** 把一批文件分成图片、PDF 和别的 (别的只数个数, 用来提示)。 */
function sortFiles(all) {
  const images = all.filter(isImage);
  const documents = all.filter(isPdf);
  return { images, documents, ignored: all.length - images.length - documents.length };
}

const messageOf = (error) => (error && error.message ? error.message : String(error));

// 同源的静态文件, 由 scripts/build-ui.sh web 拷进站点的 pdfjs/。模块按绝对 URL 导入 (host.js 是原样拷贝的)。
const pdfAsset = (path) => new URL("pdfjs/" + path, document.baseURI).href;
let pdfjsReady;

function loadPdfjs() {
  pdfjsReady ??= import(pdfAsset("pdf.js")).then((lib) => {
    lib.GlobalWorkerOptions.workerSrc = pdfAsset("pdf.worker.js");
    return lib;
  });
  return pdfjsReady;
}

function openDoc(id) {
  const entry = pdfs.get(id);
  if (!entry) return Promise.reject(new Error("这个 PDF 已经关闭"));
  entry.doc ??= loadPdfjs().then((lib) => {
    entry.task = lib.getDocument({
      url: entry.url,
      wasmUrl: pdfAsset("wasm/"),
      cMapUrl: pdfAsset("cmaps/"),
      cMapPacked: true,
      standardFontDataUrl: pdfAsset("standard_fonts/"),
      iccUrl: pdfAsset("iccs/"),
      isEvalSupported: false,
      enableXfa: false,
      disableAutoFetch: true,
      // 默认由浏览器的 ImageDecoder 和 OffscreenCanvas 解码嵌在 PDF 里的 JPEG, 解出的位图留在渲染进程里不还:
      // 100 页的漫画 PDF 翻完渲染进程涨到约 1.1 GB, 之后 24 秒也不回落。关掉这两个, 改由 worker 里的 JS 解码, 用完就还。
      isImageDecoderSupported: false,
      isOffscreenCanvasSupported: false,
    });
    return entry.task.promise;
  });
  // 打开失败 (损坏、加密) 不缓存, 下一次调用重新试。
  entry.doc.catch(() => {
    if (pdfs.get(id) === entry) releaseDoc(entry);
  });
  return entry.doc;
}

/** 销毁加载任务 (终止它的 worker, 放掉读入的数据); 文件登记还在, 下次 openDoc 重新打开。 */
function releaseDoc(entry) {
  clearTimeout(entry.idle);
  entry.task?.destroy();
  entry.task = null;
  entry.doc = null;
}

/** 用文档做一件事: 做的期间不会被空闲释放。 */
async function withDoc(id, work) {
  const entry = pdfs.get(id);
  if (!entry) throw new Error("这个 PDF 已经关闭");
  entry.users++;
  clearTimeout(entry.idle);
  try {
    return await work(await openDoc(id));
  } finally {
    entry.users--;
    scheduleRelease(entry);
  }
}

function scheduleRelease(entry) {
  clearTimeout(entry.idle);
  if (!entry.doc || entry.users > 0 || overlay.current?.fileId === entry.id) return;
  entry.idle = setTimeout(() => {
    if (entry.users === 0 && overlay.current?.fileId !== entry.id) releaseDoc(entry);
  }, DOC_IDLE_MS);
}

/** 登记一个 PDF 并读出页数; 打不开就撤销登记并 reject 一句人读得懂的话。 */
async function adoptPdf(file) {
  const id = "pdf" + ++seq + "-" + Math.random().toString(36).slice(2, 8);
  pdfs.set(id, { id, file, url: URL.createObjectURL(file), task: null, doc: null, users: 0, idle: 0 });
  try {
    const pageCount = await withDoc(id, (doc) => doc.numPages);
    return { id, name: file.name, size: file.size, type: "application/pdf", pageCount };
  } catch (error) {
    closePdf(id);
    throw new Error("无法读取这个 PDF: " + messageOf(error));
  }
}

function closePdf(id) {
  const entry = pdfs.get(id);
  if (!entry) return;
  pdfs.delete(id);
  if (overlay.current?.fileId === id) disposeOverlay();
  releaseDoc(entry);
  URL.revokeObjectURL(entry.url);
}

async function pickPdf() {
  const [file] = (await chooseFiles("application/pdf,.pdf", false)).filter(isPdf);
  return { file: file ? await adoptPdf(file) : null };
}

/**
 * 把一页渲染成白底 PNG, 作为一个普通的本地文件登记, 之后和图片走同一条 upload.start。
 * 同时只有一页的 canvas 存在, toBlob 之后立刻清零, 字节不进 NSH 的 VM。
 */
async function renderPageToFile({ fileId, page, longEdge }) {
  return withDoc(fileId, async (doc) => {
    if (!Number.isInteger(page) || page < 0 || page >= doc.numPages) {
      throw new Error("页码超出范围: 第 " + (page + 1) + " 页, 共 " + doc.numPages + " 页");
    }
    const edge = Math.min(Math.max(Math.round(Number(longEdge)) || 3072, 256), 8192);
    const pdfPage = await doc.getPage(page + 1);
    const canvas = document.createElement("canvas");
    try {
      const base = pdfPage.getViewport({ scale: 1 });
      const viewport = pdfPage.getViewport({ scale: edge / Math.max(base.width, base.height) });
      canvas.width = Math.max(1, Math.round(viewport.width));
      canvas.height = Math.max(1, Math.round(viewport.height));
      await pdfPage.render({ canvas, viewport, background: "rgb(255,255,255)" }).promise;
      const blob = await new Promise((resolve, reject) =>
        canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("无法生成 PNG"))), "image/png"),
      );
      const stem = pdfs.get(fileId).file.name.replace(/\.pdf$/i, "") || "page";
      const name = stem + "-p" + String(page + 1).padStart(3, "0") + ".png";
      const [fileRef] = register([new File([blob], name, { type: "image/png" })], { dated: false });
      return { fileRef };
    } finally {
      canvas.width = canvas.height = 0;
      pdfPage.cleanup();
    }
  });
}

/**
 * 覆盖层: pdf.js 的 canvas 放进 TSX 的占位节点 (data-testid="pdf-slot") 里, 位置、裁剪、窗口缩放和层叠都由浏览器处理,
 * 译文 <Image> 是占位节点后面的兄弟, 自然盖在它上面。只画当前页; 新页画好后才换掉旧 canvas, 不闪白也不同时留两份。
 */
const overlay = {
  /** 最近一次 TSX 要求的状态; 没有要求或已拆除时为 null。 */
  current: null,
  /** 还没处理的最新要求: 连续要求只画最后一次。 */
  wanted: null,
  waiters: [],
  running: false,
  slot: null,
  canvas: null,
  /** 已经画在 canvas 上的是什么 (文档、页、尺寸、DPR); 一样就不重画。 */
  drawn: "",
  task: null,
  observer: null,
  resizeTimer: 0,
  /** 拆除时加一, 在途的渲染据此丢弃自己的结果。 */
  epoch: 0,
};

function show(request) {
  return new Promise((resolve, reject) => {
    overlay.waiters.push({ resolve, reject });
    overlay.current = request;
    overlay.wanted = request;
    // 不取消正在画的那页: 连按方向键时每页都被取消就永远画不出来, 让它画完再画最新的。
    if (!overlay.running) void runOverlay();
  });
}

async function runOverlay() {
  overlay.running = true;
  let failure = null;
  let pageCount = 0;
  try {
    while (overlay.wanted) {
      const request = overlay.wanted;
      overlay.wanted = null;
      pageCount = await applyOverlay(request);
    }
  } catch (error) {
    failure = error;
  } finally {
    overlay.running = false;
    for (const waiter of overlay.waiters.splice(0)) failure ? waiter.reject(failure) : waiter.resolve({ pageCount });
  }
}

/** NSH 在 effect 之后才把占位节点挂上页面, 等它出现。 */
async function findSlot() {
  const deadline = performance.now() + 3000;
  for (;;) {
    const slot = document.querySelector('[data-testid="pdf-slot"]');
    if (slot) return slot;
    if (performance.now() > deadline) throw new Error("找不到 PDF 显示区域");
    await new Promise((resolve) => requestAnimationFrame(resolve));
  }
}

function attachSlot(slot) {
  if (overlay.slot === slot) return;
  overlay.observer?.disconnect();
  overlay.canvas?.remove();
  overlay.canvas = null;
  overlay.drawn = "";
  overlay.slot = slot;
  overlay.observer = new ResizeObserver(() => {
    clearTimeout(overlay.resizeTimer);
    // 拖动窗口边缘时每帧都在变, 停下来再重画。
    overlay.resizeTimer = setTimeout(() => {
      if (overlay.current) show(overlay.current).catch(() => {});
    }, 120);
  });
  overlay.observer.observe(slot);
}

function applyOverlay(request) {
  const epoch = overlay.epoch;
  return withDoc(request.fileId, async (doc) => {
    const slot = await findSlot();
    if (epoch !== overlay.epoch) return doc.numPages;
    attachSlot(slot);
    if (overlay.canvas) overlay.canvas.style.visibility = request.hidden ? "hidden" : "visible";
    if (!request.hidden) await drawOverlayPage(doc, request, epoch);
    return doc.numPages;
  });
}

async function drawOverlayPage(doc, { fileId, page }, epoch) {
  const slot = overlay.slot;
  const box = slot.getBoundingClientRect();
  if (box.width < 2 || box.height < 2) return;
  const index = Math.min(Math.max(Math.trunc(page), 0), doc.numPages - 1);
  const ratio = window.devicePixelRatio || 1;
  const key = [fileId, index, Math.round(box.width), Math.round(box.height), ratio].join(":");
  if (overlay.drawn === key && overlay.canvas) {
    overlay.canvas.style.visibility = "visible";
    return;
  }
  const pdfPage = await doc.getPage(index + 1);
  const base = pdfPage.getViewport({ scale: 1 });
  const scale = Math.min((Math.min(box.width / base.width, box.height / base.height) * ratio), OVERLAY_MAX_EDGE / Math.max(base.width, base.height));
  const viewport = pdfPage.getViewport({ scale });
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(viewport.width));
  canvas.height = Math.max(1, Math.round(viewport.height));
  canvas.style.cssText = "position:absolute;inset:0;width:100%;height:100%;object-fit:contain;visibility:visible";
  const task = pdfPage.render({ canvas, viewport, background: "rgb(255,255,255)" });
  overlay.task = task;
  try {
    await task.promise;
  } catch (error) {
    // 被更新的要求取消: 丢掉这块半成品。
    canvas.width = canvas.height = 0;
    if (error && error.name === "RenderingCancelledException") return;
    throw error;
  } finally {
    if (overlay.task === task) overlay.task = null;
    pdfPage.cleanup();
  }
  if (epoch !== overlay.epoch || overlay.slot !== slot) {
    canvas.width = canvas.height = 0;
    return;
  }
  const previous = overlay.canvas;
  slot.appendChild(canvas);
  if (previous) {
    previous.remove();
    previous.width = previous.height = 0;
  }
  overlay.canvas = canvas;
  overlay.drawn = key;
}

function disposeOverlay() {
  const shown = pdfs.get(overlay.current?.fileId);
  overlay.epoch++;
  overlay.current = null;
  overlay.wanted = null;
  overlay.task?.cancel();
  clearTimeout(overlay.resizeTimer);
  overlay.observer?.disconnect();
  overlay.observer = null;
  if (overlay.canvas) {
    overlay.canvas.remove();
    overlay.canvas.width = overlay.canvas.height = 0;
  }
  overlay.canvas = null;
  overlay.slot = null;
  overlay.drawn = "";
  if (shown) scheduleRelease(shown);
}

export async function createHost(player) {
  const notice = (text) => player.hostEvent("host.notice", { text });
  /** PDF 走 pdf.added, 其余给提示。阅读器一次只开一个 PDF; 其余的不登记, 免得白占一个 object URL。 */
  const announce = async ({ documents, ignored }) => {
    if (ignored > 0) notice("已忽略 " + ignored + " 个不是图片或 PDF 的文件");
    if (documents.length === 0) return;
    if (documents.length > 1) notice("一次只打开一个 PDF, 已忽略另外 " + (documents.length - 1) + " 个");
    try {
      player.hostEvent("pdf.added", [await adoptPdf(documents[0])]);
    } catch (error) {
      notice(messageOf(error));
    }
  };
  /** 拖放和粘贴: 图片走 files.added。 */
  const add = async (items, plainFiles) => {
    const sorted = sortFiles(await filesFromTransfer(items, plainFiles));
    if (sorted.images.length > 0) player.hostEvent("files.added", register(sorted.images.slice(0, MAX_PICK)));
    await announce(sorted);
  };
  /** files.pick: 图片放在返回值里, PDF 与拖放一样走 pdf.added。 */
  const pick = async (limit, source) => {
    const chosen =
      source === "folder"
        ? (await chooseFiles("", true, true)).sort((a, b) => naturalCompare(a.webkitRelativePath || a.name, b.webkitRelativePath || b.name))
        : await chooseFiles("image/*,application/pdf,.pdf", true);
    const sorted = sortFiles(chosen);
    await announce(sorted);
    return { files: register(sorted.images.slice(0, limit)) };
  };

  if (hasDom) {
    let depth = 0;
    const hasFiles = (event) => event.dataTransfer && [...event.dataTransfer.types].includes("Files");
    addEventListener("dragenter", (event) => {
      if (!hasFiles(event)) return;
      event.preventDefault();
      if (depth++ === 0) player.hostEvent("files.dragState", { active: true });
    });
    addEventListener("dragover", (event) => {
      if (hasFiles(event)) event.preventDefault();
    });
    addEventListener("dragleave", (event) => {
      if (!hasFiles(event)) return;
      if (--depth <= 0) {
        depth = 0;
        player.hostEvent("files.dragState", { active: false });
      }
    });
    addEventListener("drop", (event) => {
      if (!hasFiles(event)) return;
      event.preventDefault();
      depth = 0;
      player.hostEvent("files.dragState", { active: false });
      // 条目要在事件返回前取出, 否则 dataTransfer 会被清空。
      const items = [...event.dataTransfer.items];
      const plain = [...event.dataTransfer.files];
      add(items, plain).catch((error) => notice("读取拖入的文件失败: " + error));
    });
    addEventListener("paste", (event) => {
      const pasted = [...(event.clipboardData?.files ?? [])];
      if (pasted.length > 0) add([], pasted);
    });
  }

  return {
    async call(name, args) {
      switch (name) {
        case "session.snapshot":
          return {
            daemons: hasDom ? [{ name: location.host, url: location.origin }] : [],
            paired: null,
            // 网页没有相册; 文件对话框里图片和 PDF 都能选, 能选目录的浏览器多一项「选择文件夹」。
            imports: hasDom && "webkitdirectory" in document.createElement("input") ? ["files", "folder", "pdf"] : ["files", "pdf"],
          };
        case "session.saveToken":
        case "session.clearToken":
          // Web 的 Token 由 TSX 存进 NSH Storage, 宿主不参与。
          return null;
        case "files.pick":
          return hasDom ? pick(Math.min(args?.limit ?? MAX_PICK, MAX_PICK), args?.source) : { files: [] };
        case "upload.start":
          return upload(args);
        case "artifact.save":
          return saveArtifact(args);
        case "pdf.pick":
          return hasDom ? pickPdf() : { file: null };
        case "pdf.open":
          // 网页的 PDF 是 File 对象和 object URL, 刷新页面就没了, 没有可以找回的东西。
          return { file: null };
        case "pdf.export":
          throw new Error("网页版不支持导出 PDF, 请用 App");
        case "pdf.renderPage":
          return renderPageToFile(args);
        case "pdf.show":
          return show(args);
        default:
          throw new Error("no host function " + name);
      }
    },
    post(name, args) {
      if (name === "files.release") for (const id of args?.ids ?? []) files.delete(id);
      else if (name === "pdf.release") closePdf(args?.fileId);
      else if (name === "pdf.dispose") disposeOverlay();
    },
  };
}