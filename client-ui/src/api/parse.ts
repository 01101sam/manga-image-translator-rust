import type { ConfigDoc } from "../domain/config";
import type { EngineState, EngineStatus, JobEntry, JobState } from "../domain/types";

/** 外部数据在这里一次性解析成内部类型, 内部代码不再重复检查形状。 */
export class ParseError extends Error {}

const JOB_STATES: readonly JobState[] = ["queued", "running", "done", "failed", "cancelled"];
const ENGINE_STATES: readonly EngineState[] = ["stopped", "starting", "running", "draining"];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parseJobState(value: unknown): JobState {
  if (typeof value === "string" && (JOB_STATES as readonly string[]).includes(value)) return value as JobState;
  throw new ParseError(`未知的任务状态: ${String(value)}`);
}

export function parseJobList(body: unknown): JobEntry[] {
  if (!isRecord(body) || !Array.isArray(body.jobs)) throw new ParseError("任务列表格式不对");
  return body.jobs.map((raw): JobEntry => {
    if (!isRecord(raw) || typeof raw.job_id !== "string") throw new ParseError("任务条目格式不对");
    return {
      jobId: raw.job_id,
      state: parseJobState(raw.state),
      positionInQueue: typeof raw.position_in_queue === "number" ? raw.position_in_queue : undefined,
      error: typeof raw.error === "string" ? raw.error : undefined,
    };
  });
}

export function parseJobDetail(body: unknown): { state: JobState; positionInQueue?: number; error?: string } {
  if (!isRecord(body)) throw new ParseError("任务详情格式不对");
  return {
    state: parseJobState(body.state),
    positionInQueue: typeof body.position_in_queue === "number" ? body.position_in_queue : undefined,
    error: typeof body.error === "string" ? body.error : undefined,
  };
}

export function parseEngineState(body: unknown): EngineState {
  const state = isRecord(body) ? body.state : undefined;
  if (typeof state === "string" && (ENGINE_STATES as readonly string[]).includes(state)) return state as EngineState;
  throw new ParseError("Engine 状态格式不对");
}

export function parseEngineStatus(body: unknown): EngineStatus {
  const state = parseEngineState(body);
  const capacity = (body as { queue_capacity?: unknown }).queue_capacity;
  if (typeof capacity === "number" && Number.isInteger(capacity) && capacity > 0) return { state, queueCapacity: capacity };
  throw new ParseError("Engine 状态缺少队列容量 (queue_capacity), Daemon 版本可能过旧");
}

export function parseConfig(body: unknown): ConfigDoc {
  if (!isRecord(body)) throw new ParseError("Config 格式不对");
  return body;
}

export function parseToken(body: unknown): string {
  if (isRecord(body) && typeof body.token === "string" && body.token !== "") return body.token;
  throw new ParseError("配对应答里没有 Token");
}

export function parseJobId(body: unknown): string {
  if (isRecord(body) && typeof body.job_id === "string") return body.job_id;
  throw new ParseError("提交应答里没有 job_id");
}
