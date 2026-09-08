import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { extname, join } from "node:path";

const ROOT = join(import.meta.dir, "..");
const PORT = Number(process.env.PORT ?? 3000);

function parsePerf(text: string) {
  const perf: { step: string; ms: number }[] = [];
  for (const line of text.split(/\r?\n/)) {
    const m = /^PERF (\S+) (\d+)$/.exec(line);
    if (m) perf.push({ step: m[1], ms: Number(m[2]) });
  }
  return perf;
}

function parseOcr(text: string) {
  for (const line of text.split(/\r?\n/)) {
    if (!line.startsWith("OCR_JSON ")) continue;
    try {
      const v = JSON.parse(line.slice(9));
      return Array.isArray(v) ? v : [];
    } catch {
      return [];
    }
  }
  return [];
}

const parsed = parsePerf("x\nPERF detector 12\nPERF ocr 3\n");
if (parsed.length !== 2 || parsed[0].ms !== 12 || parsed[1].step !== "ocr") {
  throw new Error("parsePerf");
}
const ocrParsed = parseOcr('x\nOCR_JSON [{"text":"あ","translation":"a"}]\n');
if (ocrParsed.length !== 1 || ocrParsed[0].text !== "あ") {
  throw new Error("parseOcr");
}

async function resolveBin() {
  if (process.env.RUST_BIN) return process.env.RUST_BIN;
  for (const rel of ["target/release/simple-runtime", "target/debug/simple-runtime"]) {
    const abs = join(ROOT, rel);
    if (await Bun.file(abs).exists()) return abs;
  }
  return join(ROOT, "target/release/simple-runtime");
}

async function findOutputs(dir: string) {
  let result: string | undefined;
  let mask: string | undefined;
  const glob = new Bun.Glob("**/*.{png,html,jpg,jpeg,webp,bin}");
  for await (const f of glob.scan({ cwd: dir, onlyFiles: true, dot: false })) {
    if (f.endsWith(".mask.png")) mask = join(dir, f);
    else if (!result) result = join(dir, f);
  }
  return { result, mask };
}

function mimeOf(path: string) {
  switch (extname(path).toLowerCase()) {
    case ".png":
      return "image/png";
    case ".jpg":
    case ".jpeg":
      return "image/jpeg";
    case ".webp":
      return "image/webp";
    case ".html":
      return "text/html";
    default:
      return "application/octet-stream";
  }
}

function json(data: unknown, status = 200) {
  return Response.json(data, { status });
}

async function run(req: Request) {
  const form = await req.formData();
  const image = form.get("image");
  if (!(image instanceof Blob) || image.size === 0) {
    return json({ ok: false, error: "没有选择图片" });
  }
  let settings: unknown;
  try {
    settings = JSON.parse(String(form.get("settings") ?? ""));
  } catch {
    return json({ ok: false, error: "参数 JSON 无效" });
  }

  const bin = await resolveBin();
  if (!(await Bun.file(bin).exists())) {
    return json({
      ok: false,
      error: `找不到 binary: ${bin}\n请先编译: cargo build -p simple-runtime --release`,
    });
  }

  const job = await mkdtemp(join(tmpdir(), "mit-gui-"));
  try {
    const name = image instanceof File && image.name ? image.name : "input.png";
    const ext = extname(name) || ".png";
    const inputDir = join(job, "in");
    const outputDir = join(job, "out");
    await mkdir(inputDir);
    await mkdir(outputDir);
    await Bun.write(join(inputDir, `input${ext}`), image);
    const cfg = join(job, "config.json");
    await Bun.write(cfg, JSON.stringify(settings));

    const verbose = Math.min(3, Math.max(0, Number(form.get("verbose") ?? 0) | 0));
    const args = [
      ...(verbose ? ["-" + "v".repeat(verbose)] : []),
      "--max-batch-size-ocr",
      String(form.get("max_batch_size_ocr") ?? "16"),
      "--max-batch-size-upscaler",
      String(form.get("max_batch_size_upscaler") ?? "2"),
      "cli",
      "-i",
      inputDir,
      "-o",
      outputDir,
      "-c",
      cfg,
      "--overwrite",
      ...(String(form.get("save_mask")) === "1" ? ["--save-mask"] : []),
    ];

    const t0 = performance.now();
    const proc = Bun.spawn([bin, ...args], {
      cwd: ROOT,
      stdout: "pipe",
      stderr: "pipe",
    });
    const [stdout, stderr] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
    ]);
    await proc.exited;
    const wall_ms = Math.round(performance.now() - t0);
    const perf = parsePerf(stderr);
    const ocr = parseOcr(stderr);
    const log = [stdout, stderr].filter(Boolean).join("\n");
    if (proc.exitCode !== 0) {
      return json({
        ok: false,
        error: log.trim() || `exit ${proc.exitCode}`,
        perf,
        ocr,
        wall_ms,
        log,
      });
    }
    const { result, mask } = await findOutputs(outputDir);
    if (!result) {
      return json({ ok: false, error: "没有输出文件", perf, wall_ms, log });
    }
    const data = Buffer.from(await Bun.file(result).arrayBuffer()).toString("base64");
    const maskData = mask
      ? Buffer.from(await Bun.file(mask).arrayBuffer()).toString("base64")
      : null;
    return json({
      ok: true,
      mime: mimeOf(result),
      filename: result.split("/").pop(),
      data,
      mask: maskData,
      mask_mime: mask ? "image/png" : null,
      perf,
      ocr,
      wall_ms,
      log,
    });
  } finally {
    await rm(job, { recursive: true, force: true });
  }
}

const server = Bun.serve({
  port: PORT,
  async fetch(req) {
    const url = new URL(req.url);
    if (req.method === "GET" && url.pathname === "/") {
      return new Response(Bun.file(join(import.meta.dir, "index.html")));
    }
    if (req.method === "POST" && url.pathname === "/run") return run(req);
    return new Response("not found", { status: 404 });
  },
});

console.log(`http://127.0.0.1:${server.port}`);
