import type { JobEntry, JobState } from "./types";

/** 列表里一个 Job 的显示状态。`cancelPending` 只在取消请求在途时为真。 */
export interface JobRow {
  readonly jobId: string;
  readonly state: JobState;
  readonly positionInQueue?: number;
  /** undefined 表示还没取到失败原因, 不是「没有原因」。 */
  readonly error?: string;
  readonly cancelPending: boolean;
}

export type JobEvent =
  | { readonly type: "snapshot"; readonly entry: JobEntry }
  | { readonly type: "requestCancel" }
  | { readonly type: "cancelAccepted" }
  | { readonly type: "cancelRejected" }
  /** 批量状态里找不到这个 Job (Daemon 重启过)。 */
  | { readonly type: "missing" }
  | { readonly type: "errorFetched"; readonly error: string };

export type JobCommand = { readonly type: "none" } | { readonly type: "cancel"; readonly jobId: string };

const none: JobCommand = { type: "none" };

/** Daemon 只允许取消还在排队的 Job。 */
export const canCancelJob = (state: JobState): boolean => state === "queued";

export const canOpenJob = (state: JobState): boolean => state === "done";

export function jobFromEntry(entry: JobEntry): JobRow {
  return {
    jobId: entry.jobId,
    state: entry.state,
    positionInQueue: entry.state === "queued" ? entry.positionInQueue : undefined,
    error: entry.error,
    cancelPending: false,
  };
}

export function reduceJob(row: JobRow, event: JobEvent): [JobRow, JobCommand] {
  switch (event.type) {
    case "snapshot": {
      const { entry } = event;
      const queued = entry.state === "queued";
      const next: JobRow = {
        jobId: row.jobId,
        state: entry.state,
        positionInQueue: queued ? (entry.positionInQueue ?? row.positionInQueue) : undefined,
        // 状态没变时保留已取到的失败原因, 避免每秒重取。
        error: entry.error ?? (entry.state === row.state ? row.error : undefined),
        cancelPending: row.cancelPending && queued,
      };
      return [sameRow(row, next) ? row : next, none];
    }
    case "requestCancel":
      if (!canCancelJob(row.state) || row.cancelPending) return [row, none];
      return [{ ...row, cancelPending: true }, { type: "cancel", jobId: row.jobId }];
    case "cancelAccepted":
      return [{ ...row, state: "cancelled", positionInQueue: undefined, cancelPending: false }, none];
    case "cancelRejected":
      return row.cancelPending ? [{ ...row, cancelPending: false }, none] : [row, none];
    case "missing":
      if (row.state === "queued" || row.state === "running") {
        return [
          { ...row, state: "failed", positionInQueue: undefined, cancelPending: false, error: "Daemon 上找不到这个任务, 可能已重启" },
          none,
        ];
      }
      return [row, none];
    case "errorFetched":
      return row.state === "failed" ? [{ ...row, error: event.error }, none] : [row, none];
  }
}

/** 轮询每秒都会来, 没变化就必须返回同一个引用, 这样列表只更新真正变化的行。 */
function sameRow(a: JobRow, b: JobRow): boolean {
  return (
    a.state === b.state &&
    a.positionInQueue === b.positionInQueue &&
    a.error === b.error &&
    a.cancelPending === b.cancelPending
  );
}

/** 失败原因的显示文案。 */
export function jobStatusLabel(row: JobRow): string {
  switch (row.state) {
    case "queued":
      return row.positionInQueue ? `排队中 · 第 ${row.positionInQueue} 位` : "排队中";
    case "running":
      return "翻译中";
    case "done":
      return "已完成";
    case "failed":
      return row.error ? `失败 · ${row.error}` : "失败";
    case "cancelled":
      return "已取消";
  }
}

/** UUID 的第一段, 与旧 Client 的显示一致。 */
export const shortJobId = (jobId: string): string => jobId.split("-")[0] ?? jobId;
