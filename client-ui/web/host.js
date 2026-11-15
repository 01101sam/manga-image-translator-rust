// 网页宿主: 实现 HOST_PROTOCOL.md 里的全部调用。文件字节只存在于浏览器侧 (File 对象), 从不进入 NSH 的 VM。
// 无头测试 (nsh test) 会在没有 window 的上下文里加载这个模块, 所以所有 DOM 访问都在 hasDom 之后。

const hasDom = typeof window !== "undefined" && typeof document !== "undefined";
const IMAGE_EXT = /\.(png|jpe?g|webp|bmp|gif|tiff?|heic|heif)$/i;
const MAX_PICK = 500;

/** 宿主持有的文件: id -> File。上传成功或 files.release 后释放。 */
const files = new Map();
let seq = 0;

const isImage = (file) => file.type.startsWith("image/") || IMAGE_EXT.test(file.name);

function register(list) {
  return list.map((file) => {
    const id = "w" + ++seq + "-" + Math.random().toString(36).slice(2, 8);
    files.set(id, file);
    return { id, name: file.name, size: file.size, type: file.type };
  });
}

/** 递归读取目录条目里的图片。dataTransfer 的条目必须在 drop 事件同步阶段取出, 之后才可以异步遍历。 */
async function walk(entry, out) {
  if (entry.isFile) {
    const file = await new Promise((resolve, reject) => entry.file(resolve, reject));
    out.push(file);
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
  const out = [];
  for (const entry of entries) await walk(entry, out);
  return out;
}

function pickFromDialog(limit) {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.multiple = true;
    input.accept = "image/*";
    input.style.display = "none";
    const finish = (list) => {
      input.remove();
      resolve({ files: register(list.filter(isImage).slice(0, limit)) });
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

function saveArtifact({ url, filename }) {
  const link = document.createElement("a");
  link.href = url;
  link.download = filename || "translated";
  document.body.appendChild(link);
  link.click();
  link.remove();
  return null;
}

export async function createHost(player) {
  const notice = (text) => player.hostEvent("host.notice", { text });
  const add = async (items, plainFiles) => {
    const all = await filesFromTransfer(items, plainFiles);
    const images = all.filter(isImage).slice(0, MAX_PICK);
    if (images.length < all.length) notice("已忽略 " + (all.length - images.length) + " 个不是图片的文件");
    if (images.length > 0) player.hostEvent("files.added", register(images));
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
          };
        case "session.saveToken":
        case "session.clearToken":
          // Web 的 Token 由 TSX 存进 NSH Storage, 宿主不参与。
          return null;
        case "files.pick":
          return hasDom ? pickFromDialog(Math.min(args?.limit ?? MAX_PICK, MAX_PICK)) : { files: [] };
        case "upload.start":
          return upload(args);
        case "artifact.save":
          return saveArtifact(args);
        default:
          throw new Error("no host function " + name);
      }
    },
    post(name, args) {
      if (name === "files.release") for (const id of args?.ids ?? []) files.delete(id);
    },
  };
}
