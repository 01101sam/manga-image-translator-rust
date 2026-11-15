import type { StubRequest, StubResponse } from "nsh/test";

/** 测试用的 Daemon 替身: 一个有状态的网络桩, 加上宿主 (Swift / web host.js) 一侧的处理函数。 */
export const BASE = "http://daemon.test";

export interface FakeJob {
  state: "queued" | "running" | "done" | "failed" | "cancelled";
  error?: string;
}

export interface FakeOptions {
  /** 队列容量, 超过时 POST /jobs 返回 429 (真实 Daemon 是 64)。 */
  readonly capacity?: number;
  /** GET /engine/status 报出的队列容量, 默认等于 capacity; 设得比 capacity 大, 用来逼出 429 的重试路径。 */
  readonly reportedCapacity?: number;
  /** Engine 初始状态。 */
  readonly engine?: "running" | "stopped";
  /** 每次轮询把 queued 推进成 running、running 推进成 done 的个数上限。 */
  readonly finishPerPoll?: number;
  readonly pairingKey?: string;
}

export function createFakeDaemon(options: FakeOptions = {}) {
  const capacity = options.capacity ?? 64;
  const finishPerPoll = options.finishPerPoll ?? 1000;
  const jobs = new Map<string, FakeJob>();
  const log = { maxQueued: 0, rejected429: 0, submitted: 0, pairs: [] as string[], configPuts: [] as unknown[], engineActions: [] as string[] };
  let engine = options.engine ?? "running";
  let config: Record<string, unknown> = {
    workers: 2,
    api_key: "sk-secret",
    pairing_key: options.pairingKey ?? "sam01101",
    downscale_long_edge: 4096,
    translator: { mode: "Llm", backend: "openai", base_url: null, model: "deepseek-v4-flash", thinking: true, thinking_strength: "high", web_search: false, target: "chs", unknown_future_field: 42 },
    detector: { detector: "DBNet", options: { detect_size: 2048, text_threshold: 0.5, box_threshold: 0.7, unclip_ratio: 2.3 } },
    ocr: { ocr: "Ocr48px", min_text_length: 1, prob: 0.2, ignore_bubble: 0 },
    inpainter: { inpainter: "LamaLarge", inpainting_size: 2048 },
    mask_refinement: { dilation_offset: 20, kernel_size: 3, furigana: false },
    render: { renderer: "Png", alignment: "Auto", direction: "Auto", font_size: null, font_size_offset: 0, line_spacing: null, disable_font_border: false },
    upscaler: { upscaler: null },
    tagger: { tagger: "Pixai" },
  };
  let nextJob = 1;
  let validToken = "tok-1";

  const queued = () => [...jobs.values()].filter((j) => j.state === "queued").length;
  const json = (body: unknown, status = 200): StubResponse => ({ status, body });
  const authed = (headers: Record<string, string>) => headers.authorization === "Bearer " + validToken || headers.Authorization === "Bearer " + validToken;

  function advance() {
    let budget = finishPerPoll;
    for (const job of jobs.values()) {
      if (budget <= 0) break;
      if (job.state === "running") {
        job.state = "done";
        budget--;
      }
    }
    for (const job of jobs.values()) {
      if (budget <= 0) break;
      if (job.state === "queued") {
        job.state = "running";
        budget--;
      }
    }
  }

  const network: Record<string, (request: StubRequest) => StubResponse> = {
    ["POST " + BASE + "/pair"]: (request: StubRequest) => {
      const key = (JSON.parse(request.body ?? "{}") as { key: string }).key;
      log.pairs.push(key);
      return key === config.pairing_key ? json({ token: validToken }) : json({ ok: false, error: "Pairing Key 不正确" }, 401);
    },
    ["GET " + BASE + "/jobs"]: (request: StubRequest) => {
      if (!authed(request.headers)) return json({ ok: false, error: "unauthorized" }, 401);
      advance();
      return json({ jobs: [...jobs].map(([job_id, j]) => ({ job_id, state: j.state })) });
    },
    ["GET " + BASE + "/jobs/*"]: (request: StubRequest) => {
      if (!authed(request.headers)) return json({ ok: false, error: "unauthorized" }, 401);
      const id = request.url.split("/jobs/")[1]!.split(/[/?]/)[0]!;
      const job = jobs.get(id);
      return job ? json({ state: job.state, position_in_queue: null, error: job.error ?? null }) : json({ ok: false, error: "job not found" }, 404);
    },
    ["POST " + BASE + "/jobs/*"]: (request: StubRequest) => {
      const id = request.url.split("/jobs/")[1]!.split("/")[0]!;
      const job = jobs.get(id);
      if (!job) return json({ ok: false, error: "job not found" }, 404);
      if (job.state !== "queued") return json({ ok: false, error: "job is " + job.state }, 409);
      job.state = "cancelled";
      return json({ state: "cancelled" });
    },
    ["GET " + BASE + "/engine/status"]: () => json({ state: engine, queue_capacity: options.reportedCapacity ?? capacity }),
    ["POST " + BASE + "/engine/*"]: (request: StubRequest) => {
      const action = request.url.split("/engine/")[1]!;
      log.engineActions.push(action);
      engine = action === "stop" ? "stopped" : "running";
      return json({ state: engine });
    },
    ["GET " + BASE + "/config"]: () => json(config),
    ["PUT " + BASE + "/config"]: (request: StubRequest) => {
      const next = JSON.parse(request.body ?? "{}") as Record<string, unknown>;
      log.configPuts.push(next);
      config = next;
      return json(config);
    },
  };

  /** 宿主 (host.js / Swift) 的 upload.start: 对 Daemon 做一次 POST /jobs。 */
  function submit(): { ok: true; jobId: string } | { ok: false; kind: string; message: string } {
    if (engine === "stopped") return { ok: false, kind: "engine-stopped", message: "engine is stopped" };
    if (queued() >= capacity) {
      log.rejected429++;
      return { ok: false, kind: "queue-full", message: "queue is full (capacity " + capacity + ")" };
    }
    const jobId = "job-" + nextJob++ + "-aaaa-bbbb";
    jobs.set(jobId, { state: "queued" });
    log.submitted++;
    log.maxQueued = Math.max(log.maxQueued, queued());
    return { ok: true, jobId };
  }

  return {
    network,
    jobs,
    log,
    submit,
    setEngine: (state: "running" | "stopped") => (engine = state),
    revokeTokens: () => (validToken = "tok-revoked-" + Math.random()),
    token: () => validToken,
    config: () => config,
  };
}

export type FakeDaemon = ReturnType<typeof createFakeDaemon>;

export const files = (n: number, prefix = "p") =>
  Array.from({ length: n }, (_, i) => ({ id: prefix + i, name: prefix + i + ".png", size: 1000 + i, type: "image/png" }));
