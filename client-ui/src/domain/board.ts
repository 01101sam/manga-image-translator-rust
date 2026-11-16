import { jobFromEntry, reduceJob, type JobEvent, type JobRow } from "./job";
import type { SavedJob } from "./saved";
import { fingerprintOf, type Fingerprint } from "./similar";
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

export interface JobItem extends FileFacts {
  readonly kind: "job";
  readonly key: string;
  readonly row: JobRow;
}

/**
 * 排序和显示用的三项文件信息, 导入时由宿主给出。缺失的值 (宿主没读到、旧版本存档里没有) 排在末尾。
 * hash 和 aspect 是相似图片的指纹 (见 domain/similar.ts): 随条目一直带着、一起存档, 条目成为 Job 之后新导入的图还要和它比。
 */
export interface FileFacts {
  readonly name?: string;
  readonly size?: number;
  readonly modifiedAt?: number;
  readonly hash?: string;
  readonly aspect?: number;
}

export function factsOf(item: Item): FileFacts {
  if (item.kind === "job") return { name: item.name, size: item.size, modifiedAt: item.modifiedAt, hash: item.hash, aspect: item.aspect };
  const { name, size, modifiedAt, hash, aspect } = item.file;
  return { name, size: size > 0 ? size : undefined, modifiedAt, hash, aspect };
}

/** 列表里所有条目 (不分状态) 的指纹; 已移除的不在列表里, 所以不算。 */
export function fingerprintsOf(board: Board): Fingerprint[] {
  const prints: Fingerprint[] = [];
  for (const key of board.submitOrder) {
    const item = board.items[key];
    if (item === undefined) continue;
    const { hash, aspect } = factsOf(item);
    const print = fingerprintOf(hash, aspect);
    if (print !== undefined) prints.push(print);
  }
  return prints;
}

/**
 * 列表的唯一数据源。`items` 按 key 存放, 变化的 key 由每次操作返回。两个顺序互相独立:
 * - `submitOrder` 是加入顺序, Pending Upload 按它提交给 Daemon, 排序和拖动都不动它。
 * - `displayOrder` 是界面上的顺序, 新条目追加在末尾, 用户可以一次性排序或拖动调整。
 * 两者始终是同一批 key 的两种排列。
 */
export interface Board {
  readonly submitOrder: readonly string[];
  readonly displayOrder: readonly string[];
  readonly items: Readonly<Record<string, Item>>;
}

export interface Change {
  readonly board: Board;
  /** 内容变化的 key, 供 store 只更新这些行的 signal。 */
  readonly changed: readonly string[];
  /** 新增、移除、排序或移动使显示顺序或加入顺序改变。 */
  readonly orderChanged: boolean;
}

export const emptyBoard: Board = { submitOrder: [], displayOrder: [], items: {} };

/**
 * Board 自身的不变式: 两个顺序是同一批 key 的两种排列, 每个 key 都有条目, 每个条目都在两个顺序里。
 * 违反时列表里会出现没有内容的行或格, 所以每次增删之后都要检查。返回空数组表示一致。
 */
export function boardIssues(board: Board): string[] {
  const issues: string[] = [];
  const scan = (name: string, order: readonly string[]): Set<string> => {
    const seen = new Set<string>();
    for (const key of order) {
      if (seen.has(key)) issues.push(`${name} 里的 ${key} 出现了两次`);
      seen.add(key);
      const item = board.items[key];
      if (item === undefined) issues.push(`${name} 里的 ${key} 没有条目`);
      else if (item.key !== key) issues.push(`条目 ${key} 的 key 是 ${item.key}`);
    }
    return seen;
  };
  const submit = scan("submitOrder", board.submitOrder);
  const display = scan("displayOrder", board.displayOrder);
  for (const key of Object.keys(board.items)) {
    if (!submit.has(key)) issues.push(`条目 ${key} 不在 submitOrder 里`);
    if (!display.has(key)) issues.push(`条目 ${key} 不在 displayOrder 里`);
  }
  return issues;
}

/** 不变式被破坏时的补救: 去掉没有条目的 key 和重复的 key, 把不在顺序里的条目追加在末尾。 */
export function repairBoard(board: Board): Board {
  const fix = (order: readonly string[]): string[] => {
    const seen = new Set<string>();
    const kept = order.filter((key) => board.items[key] !== undefined && !seen.has(key) && seen.add(key));
    for (const key of Object.keys(board.items)) if (!seen.has(key)) kept.push(key);
    return kept;
  };
  return { ...board, submitOrder: fix(board.submitOrder), displayOrder: fix(board.displayOrder) };
}

const LOCAL_PREFIX = "local:";

export const localKey = (fileId: string): string => LOCAL_PREFIX + fileId;

export const unchanged = (board: Board): Change => ({ board, changed: [], orderChanged: false });

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
  return {
    board: { submitOrder: [...board.submitOrder, ...keys], displayOrder: [...board.displayOrder, ...keys], items },
    changed: keys,
    orderChanged: true,
  };
}

function replaceItem(board: Board, key: string, next: Item): Change {
  if (board.items[key] === next) return unchanged(board);
  return { board: { ...board, items: { ...board.items, [key]: next } }, changed: [key], orderChanged: false };
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
    ...factsOf(item),
    row: jobFromEntry({ jobId, state: "queued" }),
  };
  const items: Record<string, Item> = { ...board.items, [jobId]: job };
  delete items[key];
  const swap = (order: readonly string[]) => order.map((k) => (k === key ? jobId : k));
  return {
    board: { submitOrder: swap(board.submitOrder), displayOrder: swap(board.displayOrder), items },
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
  const keep = (order: readonly string[]) => order.filter((key) => !doomed.has(key));
  return {
    board: { submitOrder: keep(board.submitOrder), displayOrder: keep(board.displayOrder), items },
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
 * Daemon 返回的顺序不确定 (哈希表), 所以显示顺序始终以 `board.displayOrder` 为准。
 */
export function applyJobList(board: Board, entries: readonly JobEntry[]): Change {
  const byId = new Map(entries.map((entry) => [entry.jobId, entry] as const));
  let items = board.items;
  const changed: string[] = [];
  for (const key of board.submitOrder) {
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
  return { board: { ...board, items }, changed, orderChanged: false };
}

/** 刚变成 failed 但还没取到原因的 Job, 需要单独 GET /jobs/{id} 一次。 */
export function jobsNeedingError(board: Board): string[] {
  return board.displayOrder.filter((key) => {
    const item = board.items[key];
    return item?.kind === "job" && item.row.state === "failed" && item.row.error === undefined;
  });
}

/** 本地条目的 key 里的宿主 fileId; 不是本地条目 (Job 的 id) 返回 undefined。 */
export const fileIdOfKey = (key: string): string | undefined => (key.startsWith(LOCAL_PREFIX) ? key.slice(LOCAL_PREFIX.length) : undefined);

export interface Restored {
  readonly board: Board;
  /** 存档里还没上传的文件, 宿主已经找不到了 (被系统清理、网页刷新过): 数量, 供提示。 */
  readonly lost: number;
}

/**
 * 用保存的列表、Daemon 当前的批量状态和宿主还留着的文件重建 Board, 保持保存时的顺序。
 * Daemon 已不认识的 Job 丢弃; 还没上传的文件宿主 (`found`, fileId 到文件引用) 找不回的丢弃并计数。
 * 找回的文件回到 Pending (上传中的也是: 重启时那次上传已经没有了), 被拒绝的保持被拒绝。
 */
export function restoreBoard(saved: readonly SavedJob[], entries: readonly JobEntry[], found: ReadonlyMap<string, LocalFile>): Restored {
  const byId = new Map(entries.map((entry) => [entry.jobId, entry] as const));
  const order: string[] = [];
  const items: Record<string, Item> = {};
  let lost = 0;
  for (const { id, name, size, modifiedAt, type, rejected, hash, aspect } of saved) {
    if (items[id] !== undefined) continue;
    const fileId = fileIdOfKey(id);
    if (fileId !== undefined) {
      const hosted = found.get(fileId);
      if (hosted === undefined) {
        lost++;
        continue;
      }
      const file: LocalFile = {
        ...hosted,
        name: name ?? hosted.name,
        size: size ?? hosted.size,
        type: type ?? hosted.type,
        ...(modifiedAt === undefined ? {} : { modifiedAt }),
        ...(hash === undefined ? {} : { hash }),
        ...(aspect === undefined ? {} : { aspect }),
      };
      order.push(id);
      items[id] = { kind: "pending", key: id, file, ...(rejected === undefined ? {} : { rejected }) };
      continue;
    }
    const entry = byId.get(id);
    if (!entry) continue;
    order.push(id);
    items[id] = { kind: "job", key: id, name, size, modifiedAt, hash, aspect, row: jobFromEntry(entry) };
  }
  return { board: { submitOrder: order, displayOrder: order, items }, lost };
}

/** 只恢复 Job (没有宿主可问时)。 */
export const restoreJobs = (saved: readonly SavedJob[], entries: readonly JobEntry[]): Board => restoreBoard(saved, entries, new Map()).board;

/**
 * 按显示顺序存, 重启后列表就是用户看到的样子。Job 存名字、大小、修改时间; 还没上传的文件 (上传中的当作待上传) 多存 type 和被拒绝的原因,
 * 文件本身留在宿主里, 凭 fileId 找回。
 */
export function savedItems(board: Board): SavedJob[] {
  return board.displayOrder.flatMap((key): SavedJob[] => {
    const item = board.items[key];
    if (item === undefined) return [];
    if (item.kind === "job") return [{ id: key, name: item.name, size: item.size, modifiedAt: item.modifiedAt, hash: item.hash, aspect: item.aspect }];
    const { name, size, modifiedAt, type, hash, aspect } = item.file;
    return [
      {
        id: key,
        name,
        size: size > 0 ? size : undefined,
        modifiedAt,
        type,
        hash,
        aspect,
        ...(item.kind === "pending" && item.rejected !== undefined ? { rejected: item.rejected } : {}),
      },
    ];
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
  for (const key of board.submitOrder) {
    const item = board.items[key];
    if (!item) continue;
    if (item.kind === "pending") {
      if (item.rejected === undefined) c.pending++;
      else c.failed++;
    } else if (item.kind === "uploading") c.uploading++;
    else c[item.row.state]++;
  }
  return { ...c, total: board.submitOrder.length };
}

/** 在途数: 已交给 Daemon 且还没结束的 Job, 加上正在上传的。 */
export function inFlight(board: Board): number {
  let n = 0;
  for (const key of board.submitOrder) {
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

/** 按加入顺序 (不是显示顺序) 挑出现在可以开始上传的 Pending: 受队列余量和上传并发双重限制。 */
export function planUploads(board: Board, limits: UploadLimits): string[] {
  let uploading = 0;
  for (const key of board.submitOrder) if (board.items[key]?.kind === "uploading") uploading++;
  const room = Math.min(limits.capacity - inFlight(board), limits.concurrency - uploading);
  if (room <= 0) return [];
  const plan: string[] = [];
  for (const key of board.submitOrder) {
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
  return board.displayOrder.filter((key) => {
    const item = board.items[key];
    if (item?.kind === "job") return !isActive(item.row.state);
    return item?.kind === "pending" && item.rejected !== undefined;
  });
}