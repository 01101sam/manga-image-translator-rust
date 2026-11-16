/** Daemon 对一个 Job 的生命周期划分, 与 Rust 端 `JobState` 的小写序列化一一对应。 */
export type JobState = "queued" | "running" | "done" | "failed" | "cancelled";

export type EngineState = "stopped" | "starting" | "running" | "draining";

/** GET /engine/status: Engine 状态, 加上 Daemon 队列容量 (一次最多能让多少张图在 Daemon 里排队或翻译)。 */
export interface EngineStatus {
  readonly state: EngineState;
  readonly queueCapacity: number;
}

/** 一个可连接的 Daemon。`url` 不带结尾斜杠, 例如 `http://192.168.1.5:8080`。 */
export interface DaemonRef {
  readonly name: string;
  readonly url: string;
}

/** 宿主持有的本地文件的引用, 字节永远不进 JS。`id` 由宿主生成, 只在宿主进程内有意义。 */
export interface LocalFile {
  readonly id: string;
  readonly name: string;
  /** 字节数; 宿主没读到时是 0, 排序时按「缺失」算。 */
  readonly size: number;
  readonly type: string;
  /** 文件修改时间 (毫秒时间戳), 宿主没读到时缺省。App 读文件的 contentModificationDate, Web 读 File.lastModified。 */
  readonly modifiedAt?: number;
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