/** Daemon 对一个 Job 的生命周期划分, 与 Rust 端 `JobState` 的小写序列化一一对应。 */
export type JobState = "queued" | "running" | "done" | "failed" | "cancelled";

export type EngineState = "stopped" | "starting" | "running" | "draining";

/** 一个可连接的 Daemon。`url` 不带结尾斜杠, 例如 `http://192.168.1.5:8080`。 */
export interface DaemonRef {
  readonly name: string;
  readonly url: string;
}

/** 宿主持有的本地文件的引用, 字节永远不进 JS。`id` 由宿主生成, 只在宿主进程内有意义。 */
export interface LocalFile {
  readonly id: string;
  readonly name: string;
  readonly size: number;
  readonly type: string;
}

export const isActive = (state: JobState): boolean => state === "queued" || state === "running";

export const isTerminal = (state: JobState): boolean => !isActive(state);

/** 与 `api.rs` 的 `ALLOWED_OVERRIDES` 对齐: 只传 Daemon 接受的键。 */
export interface JobOverrides {
  readonly target_lang?: string;
  readonly detector?: string;
  readonly translator?: string;
}

/** Daemon 每个 Job 一行的批量状态。`position_in_queue` 与 `error` 在 Daemon 尚未提供时缺省。 */
export interface JobEntry {
  readonly jobId: string;
  readonly state: JobState;
  readonly positionInQueue?: number;
  readonly error?: string;
}
