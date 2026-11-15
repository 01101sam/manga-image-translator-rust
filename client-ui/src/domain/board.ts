import { jobFromEntry, reduceJob, type JobEvent, type JobRow } from "./job";
import { isActive, type JobEntry, type LocalFile } from "./types";

/** 本地 Item 的三态: 还没交给 Daemon、正在上传、已成为 Job。 */
export type Item = PendingItem | UploadingItem | JobItem;

export interface PendingItem {
  readonly kind: "pending";
  readonly key: string;
  readonly file: LocalFile;
  /** 被 Daemon 判定为不可接受 (非图片、过大): 不再自动重试, 用户可移除或重试。 */
  readonly rejected?: string;
}

export interface UploadingItem {
  readonly kind: "uploading";
  readonly key: string;
  readonly file: LocalFile;
}

export interface JobItem {
  readonly kind: "job";
  readonly key: string;
  readonly row: JobRow;
  /** 来自本地文件名, 恢复列表后没有。 */
  readonly name?: string;
}

/** 列表的唯一数据源: `order` 决定显示顺序, `items` 按 key 存放, 变化的 key 由每次操作返回。 */
export interface Board {
  readonly order: readonly string[];
  readonly items: Readonly<Record<string, Item>>;
}

export interface Change {
  readonly board: Board;
  /** 内容变化的 key, 供 store 只更新这些行的 signal。 */
  readonly changed: readonly string[];
  /** 新增或移除的 key 使 order 改变。 */
  readonly orderChanged: boolean;
}

export const emptyBoard: Board = { order: [], items: {} };

export const localKey = (fileId: string): string => `local:${fileId}`;

const unchanged = (board: Board): Change => ({ board, changed: [], orderChanged: false });

export function addFiles(board: Board, files: readonly LocalFile[]): Change {
  const fresh = files.filter((file) => board.items[localKey(file.id)] === undefined);
  if (fresh.length === 0) return unchanged(board);
  const items: Record<string, Item> = { ...board.items };
  const keys: string[] = [];
  for (const file of fresh) {
    const key = localKey(file.id);
    items[key] = { kind: "pending", key, file };
    keys.push(key);
  }
  return { board: { order: [...board.order, ...keys], items }, changed: keys, orderChanged: true };
}

function replaceItem(board: Board, key: string, next: Item): Change {
  if (board.items[key] === next) return unchanged(board);
  return { board: { order: board.order, items: { ...board.items, [key]: next } }, changed: [key], orderChanged: false };
}

export function startUpload(board: Board, key: string): Change {
  const item = board.items[key];
  if (item?.kind !== "pending" || item.rejected !== undefined) return unchanged(board);
  return replaceItem(board, key, { kind: "uploading", key, file: item.file });
}

/** 上传成功: 同一位置的 Item 换成 Job, key 从本地 id 变成 jobId。 */
export function uploadSucceeded(board: Board, key: string, jobId: string): Change {
  const item = board.items[key];
  if (item?.kind !== "uploading") return unchanged(board);
  const existing = board.items[jobId];
  if (existing !== undefined) {
    return removeKeys(board, [key]);
  }
  const job: JobItem = {
    kind: "job",
    key: jobId,
    name: item.file.name,
    row: jobFromEntry({ jobId, state: "queued" }),
  };
  const items: Record<string, Item> = { ...board.items, [jobId]: job };
  delete items[key];
  return {
    board: { order: board.order.map((k) => (k === key ? jobId : k)), items },
    changed: [jobId],
    orderChanged: true,
  };
}

/** Queue 满、Engine 停止、网络错误: 回到 Pending, 由调度器稍后重试。 */
export function uploadRequeued(board: Board, key: string): Change {
  const item = board.items[key];
  if (item?.kind !== "uploading") return unchanged(board);
  return replaceItem(board, key, { kind: "pending", key, file: item.file });
}

export function uploadRejected(board: Board, key: string, message: string): Change {
  const item = board.items[key];
  if (item?.kind !== "uploading") return unchanged(board);
  return replaceItem(board, key, { kind: "pending", key, file: item.file, rejected: message });
}

export function retryRejected(board: Board, key: string): Change {
  const item = board.items[key];
  if (item?.kind !== "pending" || item.rejected === undefined) return unchanged(board);
  return replaceItem(board, key, { kind: "pending", key, file: item.file });
}

export function removeKeys(board: Board, keys: readonly string[]): Change {
  const doomed = new Set(keys.filter((key) => board.items[key] !== undefined));
  if (doomed.size === 0) return unchanged(board);
  const items: Record<string, Item> = { ...board.items };
  for (const key of doomed) delete items[key];
  return {
    board: { order: board.order.filter((key) => !doomed.has(key)), items },
    changed: [...doomed],
    orderChanged: true,
  };
}

/** 对一个 Job Item 套用 reduceJob, 并把新状态写回。 */
export function applyJobEvent(board: Board, jobId: string, event: JobEvent): { change: Change; command: ReturnType<typeof reduceJob>[1] } {
  const item = board.items[jobId];
  if (item?.kind !== "job") return { change: unchanged(board), command: { type: "none" } };
  const [row, command] = reduceJob(item.row, event);
  if (row === item.row) return { change: unchanged(board), command };
  return { change: replaceItem(board, jobId, { ...item, row }), command };
}

/**
 * 套用一次 `GET /jobs` 的批量状态。没变化的行保持同一个引用, 返回的 `changed` 只含真正变化的 key。
 * Daemon 返回的顺序不确定 (哈希表), 所以显示顺序始终以 `board.order` 为准。
 */
export function applyJobList(board: Board, entries: readonly JobEntry[]): Change {
  const byId = new Map(entries.map((entry) => [entry.jobId, entry] as const));
  let items = board.items;
  const changed: string[] = [];
  for (const key of board.order) {
    const item = board.items[key];
    if (item?.kind !== "job") continue;
    const entry = byId.get(key);
    const [row] = reduceJob(item.row, entry ? { type: "snapshot", entry } : { type: "missing" });
    if (row !== item.row) {
      if (items === board.items) items = { ...board.items };
      (items as Record<string, Item>)[key] = { ...item, row };
      changed.push(key);
    }
  }
  if (changed.length === 0) return unchanged(board);
  return { board: { order: board.order, items }, changed, orderChanged: false };
}

/** 刚变成 failed 但还没取到原因的 Job, 需要单独 GET /jobs/{id} 一次。 */
export function jobsNeedingError(board: Board): string[] {
  return board.order.filter((key) => {
    const item = board.items[key];
    return item?.kind === "job" && item.row.state === "failed" && item.row.error === undefined;
  });
}

/** 保存下来的一个 Job: id 和当时的文件名 (恢复后列表仍显示文件名)。 */
export interface SavedJob {
  readonly id: string;
  readonly name?: string;
}

/** 用保存的列表和 Daemon 当前的批量状态重建 Job Item, 保持保存时的顺序, Daemon 已不认识的丢弃。 */
export function restoreJobs(saved: readonly SavedJob[], entries: readonly JobEntry[]): Board {
  const byId = new Map(entries.map((entry) => [entry.jobId, entry] as const));
  const order: string[] = [];
  const items: Record<string, Item> = {};
  for (const { id, name } of saved) {
    const entry = byId.get(id);
    if (!entry || items[id] !== undefined) continue;
    order.push(id);
    items[id] = { kind: "job", key: id, name, row: jobFromEntry(entry) };
  }
  return { order, items };
}

export function savedJobs(board: Board): SavedJob[] {
  return board.order.flatMap((key) => {
    const item = board.items[key];
    return item?.kind === "job" ? [{ id: key, name: item.name }] : [];
  });
}

export interface Counts {
  readonly pending: number;
  readonly uploading: number;
  readonly queued: number;
  readonly running: number;
  readonly done: number;
  readonly failed: number;
  readonly cancelled: number;
  readonly total: number;
}

export function countItems(board: Board): Counts {
  const c = { pending: 0, uploading: 0, queued: 0, running: 0, done: 0, failed: 0, cancelled: 0 };
  for (const key of board.order) {
    const item = board.items[key];
    if (!item) continue;
    if (item.kind === "pending") {
      if (item.rejected === undefined) c.pending++;
      else c.failed++;
    } else if (item.kind === "uploading") c.uploading++;
    else c[item.row.state]++;
  }
  return { ...c, total: board.order.length };
}

/** 在途数: 已交给 Daemon 且还没结束的 Job, 加上正在上传的。 */
export function inFlight(board: Board): number {
  let n = 0;
  for (const key of board.order) {
    const item = board.items[key];
    if (item?.kind === "uploading") n++;
    else if (item?.kind === "job" && isActive(item.row.state)) n++;
  }
  return n;
}

export interface UploadLimits {
  /** Daemon 队列容量。 */
  readonly capacity: number;
  /** 同时上传的文件数。 */
  readonly concurrency: number;
}

/** 按顺序挑出现在可以开始上传的 Pending: 受队列余量和上传并发双重限制。 */
export function planUploads(board: Board, limits: UploadLimits): string[] {
  let uploading = 0;
  for (const key of board.order) if (board.items[key]?.kind === "uploading") uploading++;
  const room = Math.min(limits.capacity - inFlight(board), limits.concurrency - uploading);
  if (room <= 0) return [];
  const plan: string[] = [];
  for (const key of board.order) {
    if (plan.length >= room) break;
    const item = board.items[key];
    if (item?.kind === "pending" && item.rejected === undefined) plan.push(key);
  }
  return plan;
}

export const UPLOAD_CONCURRENCY = 3;

/** 能从列表里移除的项: 上传中的宿主还在读, 排队和翻译中的 Job 还在 Daemon 上 (要先取消), 其余都可以。 */
export function isRemovable(item: Item | undefined): boolean {
  if (item === undefined || item.kind === "uploading") return false;
  return item.kind === "pending" || !isActive(item.row.state);
}

/** 「清除已完成」要移除的 key: 已结束的 Job 和被拒绝的 Pending。 */
export function clearableKeys(board: Board): string[] {
  return board.order.filter((key) => {
    const item = board.items[key];
    if (item?.kind === "job") return !isActive(item.row.state);
    return item?.kind === "pending" && item.rejected !== undefined;
  });
}
