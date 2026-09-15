import { join } from "node:path";

const ROOT = join(import.meta.dir, "..");
const PORT = Number(process.env.PORT ?? 3000);
const PORT_S = Number(process.env.SERVICE_PORT ?? 8080);

type SpawnParams = {
  verbose: number;
  max_batch_size_ocr: number;
  max_batch_size_upscaler: number;
};

type ServiceState = "stopped" | "starting" | "ready";

type ServiceRec = {
  state: ServiceState;
  pid?: number;
  port: number;
  startedAt?: number;
  spawnParams?: SpawnParams;
  lastError?: string;
  proc?: Bun.Subprocess;
};

const svc: ServiceRec = { state: "stopped", port: PORT_S };

const RING_MAX = 500;
let lineSeq = 0;
const ring: { seq: number; line: string }[] = [];

function pushLine(line: string) {
  ring.push({ seq: ++lineSeq, line });
  if (ring.length > RING_MAX) ring.shift();
}

function linesAfter(seq: number) {
  return ring
    .filter((x) => x.seq > seq)
    .map((x) => x.line)
    .join("\n");
}

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

function json(data: unknown, status = 200) {
  return Response.json(data, { status });
}

function sleep(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

function paramsEq(a?: SpawnParams, b?: SpawnParams) {
  return (
    !!a &&
    !!b &&
    a.verbose === b.verbose &&
    a.max_batch_size_ocr === b.max_batch_size_ocr &&
    a.max_batch_size_upscaler === b.max_batch_size_upscaler
  );
}

function spawnParamsFrom(form: FormData): SpawnParams {
  return {
    verbose: Math.min(3, Math.max(0, Number(form.get("verbose") ?? 0) | 0)),
    max_batch_size_ocr: Number(form.get("max_batch_size_ocr") ?? 16) || 16,
    max_batch_size_upscaler: Number(form.get("max_batch_size_upscaler") ?? 2) || 2,
  };
}

async function runCmd(cmd: string, args: string[]) {
  try {
    const p = Bun.spawn([cmd, ...args], { stdout: "pipe", stderr: "pipe" });
    const [stdout, stderr] = await Promise.all([
      new Response(p.stdout).text(),
      new Response(p.stderr).text(),
    ]);
    await p.exited;
    return { ok: p.exitCode === 0, text: stdout, err: stderr };
  } catch (e) {
    return { ok: false, text: "", err: String(e) };
  }
}

function drainStdout(proc: Bun.Subprocess) {
  const stream = proc.stdout;
  if (stream && typeof stream !== "number") void new Response(stream).text();
}

function pumpStderr(proc: Bun.Subprocess) {
  const stream = proc.stderr;
  if (!stream || typeof stream === "number") return;
  void (async () => {
    const reader = stream.getReader();
    const dec = new TextDecoder();
    let buf = "";
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        buf += dec.decode(value, { stream: true });
        const parts = buf.split(/\r?\n/);
        buf = parts.pop() ?? "";
        for (const line of parts) pushLine(line);
      }
      if (buf) pushLine(buf);
    } catch {
      return;
    }
  })();
}

async function healthOk(ms = 1000) {
  try {
    const ac = new AbortController();
    const t = setTimeout(() => ac.abort(), ms);
    const r = await fetch(`http://127.0.0.1:${PORT_S}/health`, { signal: ac.signal });
    clearTimeout(t);
    if (!r.ok) return false;
    const body = (await r.json()) as { service?: string };
    return body.service === "simple-runtime-api";
  } catch {
    return false;
  }
}

function markStopped(err?: string) {
  svc.state = "stopped";
  svc.pid = undefined;
  svc.startedAt = undefined;
  svc.proc = undefined;
  if (err) svc.lastError = err;
}

async function stopService() {
  const proc = svc.proc;
  if (!proc) {
    markStopped();
    return;
  }
  try {
    proc.kill("SIGTERM");
  } catch {}
  const dead = await Promise.race([
    proc.exited.then(() => true),
    sleep(3000).then(() => false),
  ]);
  if (!dead) {
    try {
      proc.kill("SIGKILL");
    } catch {}
    await proc.exited.catch(() => {});
  }
  if (svc.proc === proc) markStopped();
}

async function spawnService(params: SpawnParams) {
  const bin = await resolveBin();
  if (!(await Bun.file(bin).exists())) {
    const msg = `找不到 binary: ${bin}\n请先编译: cargo build -p simple-runtime --release`;
    markStopped(msg);
    throw new Error(msg);
  }
  const args = [
    ...(params.verbose ? ["-" + "v".repeat(params.verbose)] : []),
    "--max-batch-size-ocr",
    String(params.max_batch_size_ocr),
    "--max-batch-size-upscaler",
    String(params.max_batch_size_upscaler),
    "api",
    "--host",
    "127.0.0.1",
    "--port",
    String(PORT_S),
  ];
  const proc = Bun.spawn([bin, ...args], {
    cwd: ROOT,
    stdout: "pipe",
    stderr: "pipe",
  });
  svc.state = "starting";
  svc.pid = proc.pid;
  svc.port = PORT_S;
  svc.startedAt = Date.now();
  svc.spawnParams = params;
  svc.proc = proc;
  svc.lastError = undefined;
  drainStdout(proc);
  pumpStderr(proc);
  void proc.exited.then((code) => {
    if (svc.proc === proc) {
      markStopped(code === 0 || code === null ? undefined : `服务进程退出 ${code}`);
    }
  });
}

async function waitUntilReady() {
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    if (svc.state === "stopped") {
      throw new Error(svc.lastError || "服务进程已退出");
    }
    if (await healthOk(400)) {
      svc.state = "ready";
      return;
    }
    await sleep(500);
  }
  await stopService();
  markStopped("服务在 60 秒内未就绪");
  throw new Error("服务在 60 秒内未就绪");
}

async function ensureService(params: SpawnParams) {
  if (svc.state === "ready" && !paramsEq(svc.spawnParams, params)) {
    await stopService();
  }
  if (svc.state === "ready") {
    if (await healthOk(1000)) return;
    await stopService();
  }
  if (svc.state === "starting") {
    await waitUntilReady();
    return;
  }
  await spawnService(params);
  await waitUntilReady();
}

type GpuCap = { kind: "ioreg" } | { kind: "none"; note: string };
let gpuCap: GpuCap | undefined;

function parseIoregGpu(text: string) {
  const m = text.match(/"Device Utilization %"\s*=\s*(\d+)/);
  return m ? Number(m[1]) : null;
}

async function probeGpu(): Promise<GpuCap> {
  const ioreg = await runCmd("ioreg", ["-r", "-c", "IOAccelerator"]);
  if (parseIoregGpu(ioreg.text) != null) return { kind: "ioreg" };
  const pm = await runCmd("sudo", ["-n", "powermetrics", "--samplers", "gpu_power", "-n", "1", "-i", "1"]);
  if (pm.ok) {
    return {
      kind: "none",
      note: "ioreg 无 Device Utilization %，powermetrics 可用但不用于 3 秒轮询",
    };
  }
  return {
    kind: "none",
    note: "无法读取 GPU 占用：ioreg 无利用率字段，powermetrics 需要免密 sudo",
  };
}

async function readGpu() {
  gpuCap ??= await probeGpu();
  if (gpuCap.kind === "none") return { available: false, note: gpuCap.note };
  const ioreg = await runCmd("ioreg", ["-r", "-c", "IOAccelerator"]);
  const percent = parseIoregGpu(ioreg.text);
  if (percent == null) return { available: false, note: "ioreg 本次未读到 Device Utilization %" };
  return { available: true, percent };
}

function parsePsTime(s: string) {
  const t = s.trim();
  if (!t) return 0;
  const dash = t.split("-");
  let rest = t;
  let days = 0;
  if (dash.length === 2) {
    days = Number(dash[0]);
    rest = dash[1];
  }
  const parts = rest.split(":").map(Number);
  if (parts.some((n) => Number.isNaN(n))) return 0;
  if (parts.length === 3) return days * 86400 + parts[0] * 3600 + parts[1] * 60 + parts[2];
  if (parts.length === 2) return days * 86400 + parts[0] * 60 + parts[1];
  return 0;
}

let cpuSample: { pid: number; cputime: number; ts: number } | null = null;

async function readProcRes(pid: number) {
  const ps = await runCmd("ps", ["-o", "time=,rss=", "-p", String(pid)]);
  const cols = ps.text.trim().split(/\s+/);
  const cputime = parsePsTime(cols[0] ?? "");
  const rssKb = Number(cols[1] ?? "");
  const rss_bytes = Number.isFinite(rssKb) ? Math.round(rssKb * 1024) : null;
  const now = Date.now();
  let cpu_pct: number | null = null;
  if (cpuSample && cpuSample.pid === pid) {
    const dt = (now - cpuSample.ts) / 1000;
    if (dt > 0) cpu_pct = Math.max(0, ((cputime - cpuSample.cputime) / dt) * 100);
  }
  cpuSample = { pid, cputime, ts: now };
  return { cpu_pct, rss_bytes };
}

async function serviceStatus() {
  const running = svc.state === "ready";
  const uptime_s =
    running && svc.startedAt ? Math.round((Date.now() - svc.startedAt) / 1000) : null;
  let cpu_pct: number | null = null;
  let rss_bytes: number | null = null;
  let gpu: { available: boolean; percent?: number; note?: string } = { available: false };
  if (svc.pid && (svc.state === "ready" || svc.state === "starting")) {
    const res = await readProcRes(svc.pid);
    cpu_pct = res.cpu_pct;
    rss_bytes = res.rss_bytes;
    gpu = await readGpu();
  }
  return json({
    state: svc.state,
    running,
    pid: svc.pid ?? null,
    uptime_s,
    cpu_pct,
    rss_bytes,
    gpu,
    spawn_params: svc.spawnParams ?? null,
  });
}

let runTail: Promise<unknown> = Promise.resolve();
function serializeRun<T>(fn: () => Promise<T>): Promise<T> {
  const done = runTail.then(fn, fn);
  runTail = done.then(
    () => {},
    () => {},
  );
  return done;
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

  const params = spawnParamsFrom(form);
  try {
    await ensureService(params);
  } catch (e) {
    return json({ ok: false, error: e instanceof Error ? e.message : String(e) });
  }

  const out = new FormData();
  const name = image instanceof File && image.name ? image.name : "input.png";
  out.append("image", image, name);
  out.append("settings", JSON.stringify(settings));
  if (String(form.get("save_mask")) === "1") out.append("save_mask", "1");

  const mark = lineSeq;
  let res: Response;
  try {
    res = await fetch(`http://127.0.0.1:${PORT_S}/translate`, {
      method: "POST",
      body: out,
    });
  } catch (e) {
    return json({ ok: false, error: `转发 /translate 失败: ${e instanceof Error ? e.message : e}` });
  }

  const drainUntil = Date.now() + 300;
  while (Date.now() < drainUntil) {
    const t = linesAfter(mark);
    if (t.includes("PERF render") || t.includes("OCR_JSON ")) break;
    await sleep(20);
  }
  const windowText = linesAfter(mark);
  const perf = parsePerf(windowText);
  const parsedOcr = parseOcr(windowText);
  let body: Record<string, unknown>;
  try {
    body = (await res.json()) as Record<string, unknown>;
  } catch {
    return json({
      ok: false,
      error: `服务返回了非 JSON（HTTP ${res.status}）`,
      perf,
      log: windowText,
    });
  }
  const ocr = Array.isArray(body.ocr) ? body.ocr : parsedOcr;
  return json({ ...body, perf, ocr, log: windowText }, res.status);
}

const server = Bun.serve({
  port: PORT,
  async fetch(req) {
    const url = new URL(req.url);
    if (req.method === "GET" && url.pathname === "/") {
      return new Response(Bun.file(join(import.meta.dir, "index.html")));
    }
    if (req.method === "GET" && url.pathname === "/service/status") {
      return serviceStatus();
    }
    if (req.method === "POST" && url.pathname === "/run") {
      return serializeRun(() => run(req));
    }
    return new Response("not found", { status: 404 });
  },
});

console.log(`http://127.0.0.1:${server.port}`);
