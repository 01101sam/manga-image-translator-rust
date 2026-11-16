// 网页宿主: 实现 HOST_PROTOCOL.md 里的全部调用。文件字节只存在于浏览器侧 (File 对象), 从不进入 NSH 的 VM。
// 无头测试 (nsh test) 会在没有 window 的上下文里加载这个模块, 所以所有 DOM 访问都在 hasDom 之后。

const hasDom = typeof window !== "undefined" && typeof document !== "undefined";
const IMAGE_EXT = /\.(png|jpe?g|webp|bmp|gif|tiff?|heic|heif)$/i;
const MAX_PICK = 500;

/** 宿主持有的文件: id -> File。上传成功或 files.release 后释放。 */
const files = new Map();
let seq = 0;

const isImage = (file) => file.type.startsWith("image/") || IMAGE_EXT.test(file.name);

/**
 * Daemon 的 image crate 按内容能解码的格式 (App 一侧的 ImagePrep.daemonReadable 同一份清单)。别的 (HEIC、HEIF、AVIF、JPEG 2000 等) 不行,
 * 所以浏览器能解码的话在这里转成 JPEG; 浏览器也解不了就原样交出去, Daemon 会拒绝, 列表里显示「格式不支持」。
 */
const DAEMON_TYPES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp", "image/tiff", "image/x-icon", "image/vnd.microsoft.icon"]);
const DAEMON_EXT = /\.(png|jpe?g|gif|webp|bmp|tiff?|ico)$/i;
const JPEG_QUALITY = 0.92;

/** 浏览器有时不给 HEIC 的 type (空字符串), 所以 type 不在清单里、扩展名也不在时才转。 */
const needsConversion = (file) => !DAEMON_TYPES.has(file.type) && !DAEMON_EXT.test(file.name);

/**
 * 转成同名 (扩展名换成 .jpg) 的 JPEG。createImageBitmap 默认按 EXIF 方向解码, 方向于是烘进像素 (Daemon 不读 EXIF 方向)。
 * 一次解码一张, 画完立刻放掉位图和 canvas。任何一步失败都返回原文件。
 */
async function convertToJpeg(file) {
  if (typeof createImageBitmap !== "function") return file;
  let bitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    return file;
  }
  const canvas = document.createElement("canvas");
  try {
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    const context = canvas.getContext("2d");
    context.fillStyle = "#fff";
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.drawImage(bitmap, 0, 0);
    const blob = await new Promise((resolve) => canvas.toBlob(resolve, "image/jpeg", JPEG_QUALITY));
    if (!blob) return file;
    const stem = file.name.replace(/\.[^./\\]*$/, "") || "image";
    return new File([blob], stem + ".jpg", { type: "image/jpeg", lastModified: file.lastModified });
  } catch {
    return file;
  } finally {
    bitmap.close();
    canvas.width = canvas.height = 0;
  }
}

// ---- 相似图片的指纹: 256 位 dHash, 与 App 的 ImageFingerprint.swift 逐行对应 (算法说明在那里), 全部整数运算 ----

const FP_COLUMNS = 17;
const FP_ROWS = 16;
/** 左边比右边亮多少灰度级才记 1; 与 ImageFingerprint.margin 一致。 */
const FP_MARGIN = 3;

/** RGBA8 (不透明, 4 字节一个像素) 缩成 17x16 的灰度格子。 */
export function fingerprintGrid(rgba, width, height) {
  const grid = new Uint8Array(FP_COLUMNS * FP_ROWS);
  if (!(width > 0 && height > 0) || rgba.length < width * height * 4) return grid;
  for (let cy = 0; cy < FP_ROWS; cy++) {
    const y0 = Math.min(Math.floor((cy * height) / FP_ROWS), height - 1);
    const y1 = Math.max(Math.floor(((cy + 1) * height) / FP_ROWS), y0 + 1);
    for (let cx = 0; cx < FP_COLUMNS; cx++) {
      const x0 = Math.min(Math.floor((cx * width) / FP_COLUMNS), width - 1);
      const x1 = Math.max(Math.floor(((cx + 1) * width) / FP_COLUMNS), x0 + 1);
      let sum = 0;
      for (let y = y0; y < y1; y++) {
        let index = (y * width + x0) * 4;
        for (let x = x0; x < x1; x++) {
          sum += (77 * rgba[index] + 150 * rgba[index + 1] + 29 * rgba[index + 2] + 128) >> 8;
          index += 4;
        }
      }
      const count = (y1 - y0) * (x1 - x0);
      grid[cy * FP_COLUMNS + cx] = Math.floor((sum + Math.floor(count / 2)) / count);
    }
  }
  return grid;
}

/** 17x16 灰度的 256 位差分哈希, 64 个小写十六进制字符。 */
export function fingerprintHash(grid) {
  const bytes = new Uint8Array(((FP_COLUMNS - 1) * FP_ROWS) / 8);
  let position = 0;
  for (let row = 0; row < FP_ROWS; row++) {
    for (let column = 0; column < FP_COLUMNS - 1; column++) {
      if (grid[row * FP_COLUMNS + column] > grid[row * FP_COLUMNS + column + 1] + FP_MARGIN) bytes[position >> 3] |= 0x80 >> (position & 7);
      position++;
    }
  }
  return [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

export const fingerprintHex = (rgba, width, height) => fingerprintHash(fingerprintGrid(rgba, width, height));

const THUMB_EDGE = 256;
/** data URI 的字符数上限, 与 App 的 ImagePrep.thumbnailBudget 一样: 它随文件引用进 NSH 的 JS 堆。 */
const THUMB_BUDGET = 16_000;

/** 把一个图像源 (位图或 canvas) 缩成长边不超过 edge 的 JPEG data URI; 超过预算返回 null。 */
function thumbUri(source, width, height, edge) {
  const scale = Math.min(1, edge / Math.max(width, height));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(width * scale));
  canvas.height = Math.max(1, Math.round(height * scale));
  try {
    const context = canvas.getContext("2d");
    context.fillStyle = "#fff";
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.imageSmoothingQuality = "medium";
    context.drawImage(source, 0, 0, canvas.width, canvas.height);
    for (const quality of [0.7, 0.5, 0.35]) {
      const uri = canvas.toDataURL("image/jpeg", quality);
      if (uri.length <= THUMB_BUDGET) return uri;
    }
    return null;
  } finally {
    canvas.width = canvas.height = 0;
  }
}

/** 先 256, 压不进预算就缩到 192、128 (App 一侧同样的阶梯)。 */
function thumbFrom(source, width, height) {
  for (const edge of [THUMB_EDGE, 192, 128]) {
    const uri = thumbUri(source, width, height, edge);
    if (uri) return uri;
  }
  return undefined;
}

/** 指纹: 把图像源缩到长边不超过 THUMB_EDGE 的 canvas, 白底合成, 读出像素算 dHash。位图已经是这个大小时就是 1:1。 */
function hashOf(source, width, height) {
  const scale = Math.min(1, THUMB_EDGE / Math.max(width, height));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(width * scale));
  canvas.height = Math.max(1, Math.round(height * scale));
  try {
    const context = canvas.getContext("2d", { willReadFrequently: true });
    context.fillStyle = "#fff";
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.imageSmoothingQuality = "medium";
    context.drawImage(source, 0, 0, canvas.width, canvas.height);
    return fingerprintHex(context.getImageData(0, 0, canvas.width, canvas.height).data, canvas.width, canvas.height);
  } finally {
    canvas.width = canvas.height = 0;
  }
}

/** 一个解码好的图像源 (位图或 canvas) 的缩略图、指纹和宽高 (宽高只用来比宽高比); 任何一项失败只是没有那一项。 */
function analyzeSource(source, width, height) {
  const analysis = { width, height };
  try {
    analysis.thumb = thumbFrom(source, width, height);
  } catch {}
  try {
    analysis.hash = hashOf(source, width, height);
  } catch {}
  return analysis;
}

/**
 * 列表用的小缩略图和相似图片的指纹: 还没上传的条目只能显示缩略图 (NSH 的 Image 不读 File), 指纹让 TSX 跳过重复的图。
 * 两者用同一次解码: createImageBitmap 带 resizeWidth, 浏览器解码时就缩小。浏览器解不了 (比如 HEIC) 返回空对象, 条目照常显示名字和状态。
 */
async function analyze(file) {
  if (typeof createImageBitmap !== "function") return {};
  let bitmap;
  try {
    bitmap = await createImageBitmap(file, { resizeWidth: THUMB_EDGE, resizeQuality: "medium" });
  } catch {
    return {};
  }
  try {
    return analyzeSource(bitmap, bitmap.width, bitmap.height);
  } finally {
    bitmap.close();
  }
}

/**
 * 登记一批文件, 一次只处理一个 (解码要占内存)。`dated` 为假时不带修改时间: PDF 渲染出来的页是刚生成的, `lastModified` 只是渲染的那一刻,
 * 不是用户的文件日期。`prepare` 为假时不转码也不自己分析 (渲染出来的 PNG 页), 缩略图和指纹由调用方用 `analysis` 给。
 */
async function register(list, { dated = true, prepare = true, analysis: given } = {}) {
  const refs = [];
  for (const original of list) {
    const file = prepare && hasDom && needsConversion(original) ? await convertToJpeg(original) : original;
    const id = "w" + ++seq + "-" + Math.random().toString(36).slice(2, 8);
    files.set(id, file);
    const ref = { id, name: file.name, size: file.size, type: file.type };
    if (dated && file.lastModified > 0) ref.modifiedAt = file.lastModified;
    const analysis = given ?? (prepare && hasDom ? await analyze(file) : {});
    if (analysis.thumb) ref.thumb = analysis.thumb;
    if (analysis.hash) ref.hash = analysis.hash;
    if (analysis.width > 0 && analysis.height > 0) {
      ref.width = analysis.width;
      ref.height = analysis.height;
    }
    refs.push(ref);
  }
  return refs;
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

// ---- SHA-256: 同一个 PDF 再导入时认出来。http 的局域网页面不是安全上下文, 没有 crypto.subtle, 所以自己算; 按块读文件, 不整个读进内存 ----

const SHA_K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
  0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
  0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
  0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

class Sha256 {
  constructor() {
    this.state = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    this.block = new Uint8Array(64);
    this.used = 0;
    this.bytes = 0;
    this.schedule = new Int32Array(64);
  }

  update(data) {
    this.bytes += data.length;
    let index = 0;
    if (this.used > 0) {
      const take = Math.min(64 - this.used, data.length);
      this.block.set(data.subarray(0, take), this.used);
      this.used += take;
      index = take;
      if (this.used < 64) return;
      this.compress(this.block, 0);
      this.used = 0;
    }
    for (; index + 64 <= data.length; index += 64) this.compress(data, index);
    if (index < data.length) {
      this.block.set(data.subarray(index), 0);
      this.used = data.length - index;
    }
  }

  compress(data, offset) {
    const w = this.schedule;
    for (let t = 0; t < 16; t++) {
      const i = offset + t * 4;
      w[t] = (data[i] << 24) | (data[i + 1] << 16) | (data[i + 2] << 8) | data[i + 3];
    }
    for (let t = 16; t < 64; t++) {
      const x = w[t - 15];
      const y = w[t - 2];
      const s0 = ((x >>> 7) | (x << 25)) ^ ((x >>> 18) | (x << 14)) ^ (x >>> 3);
      const s1 = ((y >>> 17) | (y << 15)) ^ ((y >>> 19) | (y << 13)) ^ (y >>> 10);
      w[t] = (w[t - 16] + s0 + w[t - 7] + s1) | 0;
    }
    let [a, b, c, d, e, f, g, h] = this.state;
    for (let t = 0; t < 64; t++) {
      const s1 = ((e >>> 6) | (e << 26)) ^ ((e >>> 11) | (e << 21)) ^ ((e >>> 25) | (e << 7));
      const t1 = (h + s1 + ((e & f) ^ (~e & g)) + SHA_K[t] + w[t]) | 0;
      const s0 = ((a >>> 2) | (a << 30)) ^ ((a >>> 13) | (a << 19)) ^ ((a >>> 22) | (a << 10));
      const t2 = (s0 + ((a & b) ^ (a & c) ^ (b & c))) | 0;
      h = g;
      g = f;
      f = e;
      e = (d + t1) | 0;
      d = c;
      c = b;
      b = a;
      a = (t1 + t2) | 0;
    }
    const next = [a, b, c, d, e, f, g, h];
    for (let i = 0; i < 8; i++) this.state[i] = (this.state[i] + next[i]) | 0;
  }

  hex() {
    const bits = this.bytes * 8;
    const padding = new Uint8Array((this.used < 56 ? 56 - this.used : 120 - this.used) + 8);
    padding[0] = 0x80;
    const view = new DataView(padding.buffer);
    view.setUint32(padding.length - 8, Math.floor(bits / 2 ** 32));
    view.setUint32(padding.length - 4, bits >>> 0);
    this.update(padding);
    return this.state.map((word) => (word >>> 0).toString(16).padStart(8, "0")).join("");
  }
}

export function sha256Hex(bytes) {
  const hash = new Sha256();
  hash.update(bytes);
  return hash.hex();
}

const SHA_CHUNK = 4 * 1024 * 1024;

async function fileSha256(file) {
  const hash = new Sha256();
  for (let offset = 0; offset < file.size; offset += SHA_CHUNK) {
    hash.update(new Uint8Array(await file.slice(offset, offset + SHA_CHUNK).arrayBuffer()));
  }
  return hash.hex();
}

// ---- PDF: pdf.js 覆盖层与页图渲染 (契约见 HOST_PROTOCOL.md 的「PDF 阅读器」) ----

/**
 * 宿主持有的 PDF: id -> { id, file, sha, ref, url, task, doc, users, idle }。sha 是内容的 SHA-256 (去重用), ref 是交给 TSX 的文件引用。url 是 object URL; task 是 pdf.js 的加载任务, doc 是它的文档 (Promise),
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
  // 同一个 PDF (内容一样, 不管叫什么) 再导入: 不登记第二份, 返回已有的那份, TSX 于是打开已有的。
  const sha = await fileSha256(file);
  for (const entry of pdfs.values()) if (entry.sha === sha && entry.ref) return entry.ref;
  const id = "pdf" + ++seq + "-" + Math.random().toString(36).slice(2, 8);
  pdfs.set(id, { id, file, sha, ref: null, url: URL.createObjectURL(file), task: null, doc: null, users: 0, idle: 0 });
  try {
    const pageCount = await withDoc(id, (doc) => doc.numPages);
    const ref = { id, name: file.name, size: file.size, type: "application/pdf", pageCount };
    pdfs.get(id).ref = ref;
    return ref;
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
      // 页图本来就在 canvas 里, 缩略图和指纹直接从它来, 不用再解码一遍 PNG。
      const [fileRef] = await register([new File([blob], name, { type: "image/png" })], {
        dated: false,
        prepare: false,
        analysis: analyzeSource(canvas, canvas.width, canvas.height),
      });
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
    if (sorted.images.length > 0) player.hostEvent("files.added", await register(sorted.images.slice(0, MAX_PICK)));
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
    return { files: await register(sorted.images.slice(0, limit)) };
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
        case "files.resolve":
          // 网页的文件是 File 对象, 刷新页面就没了, 没有可以找回的东西。
          return { files: [] };
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