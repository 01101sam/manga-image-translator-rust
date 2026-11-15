import type { ConfigDoc } from "../domain/config";
import type { EngineState, EngineStatus, JobEntry } from "../domain/types";
import {
  parseConfig,
  parseEngineState,
  parseEngineStatus,
  parseJobDetail,
  parseJobList,
  parseToken,
  ParseError,
} from "./parse";

export type DaemonErrorKind = "unauthorized" | "rejected" | "http" | "network" | "parse";

/** 请求失败的统一形状。`unauthorized` 表示已保存的 Token 被拒, 调用方据此回到 unpaired。 */
export class DaemonError extends Error {
  constructor(
    readonly kind: DaemonErrorKind,
    message: string,
    readonly status?: number,
  ) {
    super(message);
  }
}

export interface Api {
  pair(key: string): Promise<string>;
  jobs(): Promise<JobEntry[]>;
  job(id: string): Promise<{ error?: string }>;
  cancel(id: string): Promise<void>;
  engineStatus(): Promise<EngineStatus>;
  engineAction(action: "start" | "stop" | "restart"): Promise<EngineState>;
  getConfig(): Promise<ConfigDoc>;
  putConfig(doc: ConfigDoc): Promise<ConfigDoc>;
}

const CONTROL_TIMEOUT_MS = 15_000;
/** Engine 启停会等模型加载, 比普通请求久。 */
const ENGINE_TIMEOUT_MS = 180_000;

interface Request {
  readonly method?: "GET" | "POST" | "PUT";
  readonly body?: unknown;
  readonly token: string | null;
  readonly timeoutMs?: number;
  /** /pair 的 401 是「Key 不对」, 不是「Token 失效」。 */
  readonly unauthorizedMeans?: "revoked" | "rejected";
}

async function request(baseUrl: string, path: string, req: Request): Promise<unknown> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), req.timeoutMs ?? CONTROL_TIMEOUT_MS);
  const headers: Record<string, string> = {};
  if (req.token) headers.Authorization = `Bearer ${req.token}`;
  if (req.body !== undefined) headers["Content-Type"] = "application/json";
  let response: Response;
  try {
    response = await fetch(`${baseUrl}${path}`, {
      method: req.method ?? "GET",
      headers,
      body: req.body === undefined ? undefined : JSON.stringify(req.body),
      signal: controller.signal,
    });
  } catch (error) {
    throw new DaemonError("network", controller.signal.aborted ? "请求超时" : `无法连接 Daemon: ${describe(error)}`);
  } finally {
    clearTimeout(timer);
  }
  const text = await response.text().catch(() => "");
  let json: unknown = null;
  try {
    json = text === "" ? null : JSON.parse(text);
  } catch {
    // 非 JSON 应答在下面按状态码报错。
  }
  if (response.status === 401) {
    throw req.unauthorizedMeans === "rejected"
      ? new DaemonError("rejected", errorText(json) ?? "Pairing Key 不正确", 401)
      : new DaemonError("unauthorized", "未配对或 Token 已失效", 401);
  }
  if (!response.ok) {
    throw new DaemonError("http", errorText(json) ?? `Daemon 返回 HTTP ${response.status}`, response.status);
  }
  return json;
}

function errorText(json: unknown): string | null {
  if (typeof json === "object" && json !== null && "error" in json) {
    const error = (json as { error: unknown }).error;
    if (typeof error === "string" && error !== "") return error;
  }
  return null;
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function parsed<T>(promise: Promise<unknown>, parse: (body: unknown) => T): Promise<T> {
  const body = await promise;
  try {
    return parse(body);
  } catch (error) {
    if (error instanceof ParseError) throw new DaemonError("parse", error.message);
    throw error;
  }
}

export function createApi(baseUrl: string, token: string | null): Api {
  const call = (path: string, req: Omit<Request, "token"> = {}) => request(baseUrl, path, { ...req, token });
  return {
    pair: (key) => parsed(call("/pair", { method: "POST", body: { key }, unauthorizedMeans: "rejected" }), parseToken),
    jobs: () => parsed(call("/jobs"), parseJobList),
    job: (id) => parsed(call(`/jobs/${encodeURIComponent(id)}`), parseJobDetail),
    cancel: async (id) => {
      await call(`/jobs/${encodeURIComponent(id)}/cancel`, { method: "POST" });
    },
    engineStatus: () => parsed(call("/engine/status"), parseEngineStatus),
    engineAction: (action) =>
      parsed(call(`/engine/${action}`, { method: "POST", timeoutMs: ENGINE_TIMEOUT_MS }), parseEngineState),
    getConfig: () => parsed(call("/config"), parseConfig),
    putConfig: (doc) => parsed(call("/config", { method: "PUT", body: doc }), parseConfig),
  };
}
