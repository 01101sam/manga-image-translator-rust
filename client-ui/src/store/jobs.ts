import { batch, signal, Storage, type Signal } from "nsh";
import { DaemonError } from "../api/daemon";
import {
  addFiles,
  boardIssues,
  repairBoard,
  applyJobEvent,
  applyJobList,
  clearableKeys,
  isRemovable,
  countItems,
  emptyBoard,
  jobsNeedingError,
  planUploads,
  removeKeys,
  fileIdOfKey,
  restoreBoard,
  savedItems,
  retryRejected,
  startUpload,
  UPLOAD_CONCURRENCY,
  uploadRejected,
  uploadRequeued,
  uploadSucceeded,
  type Board,
  type Change,
  type Counts,
  type Item,
} from "../domain/board";
import { moveItem, moveToBack, moveToFront, sortBoard, type SortChoice } from "../domain/arrange";
import { adjacentDone, moveHighlight } from "../domain/nav";
import { decodeSaved, encodeSaved, type SavedJob } from "../domain/saved";
import type { LocalFile } from "../domain/types";
import { engineAcceptsJobs, engineState, queueCapacity, refreshEngine } from "./engine";
import { overrides } from "./overrides";
import { api, handleError, hostPort, session } from "./session";
import { LONG_MS, showToast } from "./toast";

/**
 * 列表状态。Board 是纯数据, 这里把它的变化翻译成 signal:
 * 每个 key 一个 signal, 轮询只写真正变化的行; `displayOrder` 只在增删、排序和移动时变。
 */
let board: Board = emptyBoard;
const itemSignals = new Map<string, Signal<Item>>();

/** 界面上的顺序 (Board.displayOrder), List 按它排列。 */
export const displayOrder = signal<readonly string[]>([]);
export const counts = signal<Counts>(countItems(board));
/** 键盘高亮的行 (key)。 */
export const highlight = signal<string | null>(null);
/** 列表上方的持续提示 (Engine 停止、连不上 Daemon)。 */
export const notice = signal<string | null>(null);

/**
 * 一个 key 的当前条目。signal 还没登记时订阅显示顺序: 登记总是和显示顺序的更新在同一个 batch 里, 之后这一行会重新读到条目,
 * 不会停在空白上。
 */
export const itemOf = (key: string): Item | undefined => {
  const entry = itemSignals.get(key);
  if (entry) return entry();
  displayOrder();
  return board.items[key];
};
export const currentBoard = (): Board => board;

/**
 * 本地文件的结局。上传成功时 Item 的 key 从 local:<fileId> 换成 jobId, 从列表外面看不出对应关系;
 * PDF 阅读器靠这个把自己渲染出来的页图和后来的 Job 对上号。
 */
export type FileOutcome =
  | { readonly type: "uploaded"; readonly fileId: string; readonly jobId: string }
  | { readonly type: "rejected"; readonly fileId: string; readonly message: string }
  | { readonly type: "removed"; readonly fileId: string };

const outcomeListeners = new Set<(outcome: FileOutcome) => void>();
const rekeyListeners = new Set<(from: string, to: string) => void>();

/**
 * 待上传项上传成功时 key 从 local:<fileId> 换成 jobId。依赖旧 key 的界面状态 (正在拖的项、打开着的条目菜单)
 * 在列表更新的同一个 batch 里先换成新 key, 不然它们会因为找不到旧 key 而放弃。
 */
export function onRekey(listener: (from: string, to: string) => void): void {
  rekeyListeners.add(listener);
}

export function onFileOutcome(listener: (outcome: FileOutcome) => void): () => void {
  outcomeListeners.add(listener);
  return () => outcomeListeners.delete(listener);
}

function emitOutcome(outcome: FileOutcome) {
  for (const listener of outcomeListeners) listener(outcome);
}

const sameCounts = (a: Counts, b: Counts) => (Object.keys(a) as (keyof Counts)[]).every((k) => a[k] === b[k]);

function commit(change: Change) {
  if (change.board === board) return;
  board = change.board;
  const issues = change.orderChanged ? boardIssues(board) : [];
  if (issues.length > 0) {
    // 正常不会到这里 (board.test.ts 对每个操作断言不变式); 万一破坏了, 修好再显示, 免得列表里出现空白行。
    board = repairBoard(board);
    showToast(`列表状态不一致, 已自动修复: ${issues[0]}`, "error");
  }
  batch(() => {
    for (const key of change.changed) {
      const item = board.items[key];
      if (item === undefined) continue;
      const existing = itemSignals.get(key);
      if (existing) existing.set(item);
      else itemSignals.set(key, signal(item));
    }
    // 被拒绝和重试改的是条目的内容, 不是顺序, 但存档里有它们。
    if (!change.orderChanged && change.changed.some((key) => board.items[key]?.kind === "pending")) schedulePersist();
    if (change.orderChanged) {
      for (const key of itemSignals.keys()) if (board.items[key] === undefined) itemSignals.delete(key);
      for (const key of board.displayOrder) if (!itemSignals.has(key)) itemSignals.set(key, signal(board.items[key]!));
      displayOrder.set(board.displayOrder);
      if (highlight() !== null && board.items[highlight()!] === undefined) highlight.set(null);
      schedulePersist();
    }
    const next = countItems(board);
    if (!sameCounts(next, counts())) counts.set(next);
  });
}

// ---- 持久化: 按显示顺序存 Job 和还没上传的文件 (id、名字、大小、修改时间), 重启或刷新后 Job 从 Daemon 恢复, 文件由宿主凭 fileId 找回 ----

let persistTimer: ReturnType<typeof setTimeout> | undefined;
const storageKey = (url: string) => `jobs:${url}`;
const MAX_PERSISTED = 2000;

function schedulePersist() {
  if (persistTimer !== undefined) clearTimeout(persistTimer);
  persistTimer = setTimeout(() => {
    const s = session();
    if (s.kind !== "paired") return;
    const waiting = unrestoredJobs.filter((row) => board.items[row.id] === undefined);
    void Storage.setItem(storageKey(s.daemon.url), encodeSaved([...waiting, ...savedItems(board)].slice(-MAX_PERSISTED)));
  }, 400);
}

// ---- 操作 ----

export function addLocalFiles(files: readonly LocalFile[]) {
  commit(addFiles(board, files));
  pump();
}

export function removeItems(keys: readonly string[]) {
  const removable = keys.filter((key) => isRemovable(board.items[key]));
  const releasable = removable.flatMap((key) => {
    const item = board.items[key];
    return item?.kind === "pending" ? [item.file.id] : [];
  });
  hostPort.release(releasable);
  commit(removeKeys(board, removable));
  for (const fileId of releasable) emitOutcome({ type: "removed", fileId });
}

// ---- 显示顺序: 只影响界面, 不影响提交给 Daemon 的先后 ----

/** 一次性排序: 把现有的所有条目排一遍, 之后加入的条目照常追加在末尾。 */
export function sortItems(choice: SortChoice) {
  commit(sortBoard(board, choice));
}

export function moveItemTo(key: string, toIndex: number) {
  commit(moveItem(board, key, toIndex));
}

export const moveItemToFront = (key: string) => commit(moveToFront(board, key));

export const moveItemToBack = (key: string) => commit(moveToBack(board, key));

/** 条目当前在显示顺序里的位置, 不在列表里返回 -1。 */
export const displayIndexOf = (key: string): number => board.displayOrder.indexOf(key);

export function clearFinished() {
  removeItems(clearableKeys(board));
}

export function retryItem(key: string) {
  commit(retryRejected(board, key));
  pump();
}

export async function cancelItem(key: string): Promise<void> {
  const item = board.items[key];
  if (item?.kind === "pending") {
    removeItems([key]);
    return;
  }
  if (item?.kind !== "job") return;
  const { change, command } = applyJobEvent(board, key, { type: "requestCancel" });
  commit(change);
  const client = api();
  if (command.type !== "cancel" || !client) return;
  try {
    await client.cancel(command.jobId);
    commit(applyJobEvent(board, key, { type: "cancelAccepted" }).change);
  } catch (error) {
    commit(applyJobEvent(board, key, { type: "cancelRejected" }).change);
    if (!handleError(error)) showToast(error instanceof Error ? error.message : String(error), "error");
  }
}

export function moveSelection(delta: number) {
  const keys = board.displayOrder;
  const current = highlight() === null ? null : keys.indexOf(highlight()!);
  const next = moveHighlight(keys.length, current === -1 ? null : current, delta);
  highlight.set(next === null ? null : keys[next]!);
  return next;
}

export const adjacentDoneKey = (current: string | null, delta: number) => adjacentDone(board, current, delta);

// ---- 上传调度 ----

let pausedUntil = 0;
let pumpTimer: ReturnType<typeof setTimeout> | undefined;

function pausePump(ms: number) {
  pausedUntil = Date.now() + ms;
  if (pumpTimer !== undefined) clearTimeout(pumpTimer);
  pumpTimer = setTimeout(pump, ms + 20);
}

/** 在有变化的地方 (加文件、上传结束、轮询回来、Engine 状态变) 调用; 幂等, 只补满空位。 */
export function pump() {
  const s = session();
  if (s.kind !== "paired") return;
  if (Date.now() < pausedUntil) return;
  const capacity = queueCapacity();
  if (capacity === null || !engineAcceptsJobs()) return;
  for (const key of planUploads(board, { capacity, concurrency: UPLOAD_CONCURRENCY })) {
    commit(startUpload(board, key));
    void upload(key);
  }
}

async function upload(key: string): Promise<void> {
  const item = board.items[key];
  const s = session();
  if (item?.kind !== "uploading" || s.kind !== "paired") return;
  const outcome = await hostPort.upload({ file: item.file, baseUrl: s.daemon.url, token: s.token, overrides: overrides() });
  if (outcome.ok) {
    batch(() => {
      for (const listener of rekeyListeners) listener(key, outcome.jobId);
      commit(uploadSucceeded(board, key, outcome.jobId));
    });
    emitOutcome({ type: "uploaded", fileId: item.file.id, jobId: outcome.jobId });
  } else {
    switch (outcome.kind) {
      case "rejected":
        // 文件留在宿主里, 「重试」才有东西可传; 用户移除这一项时才释放 (removeItems)。
        commit(uploadRejected(board, key, outcome.message));
        emitOutcome({ type: "rejected", fileId: item.file.id, message: outcome.message });
        break;
      case "queue-full":
        commit(uploadRequeued(board, key));
        pausePump(1500);
        break;
      case "engine-stopped":
        commit(uploadRequeued(board, key));
        engineState.set("stopped");
        break;
      case "unauthorized":
        commit(uploadRequeued(board, key));
        handleError(new DaemonError("unauthorized", outcome.message, 401));
        break;
      case "network":
        commit(uploadRequeued(board, key));
        notice.set(`无法连接 Daemon: ${outcome.message}`);
        pausePump(3000);
        break;
    }
  }
  pump();
}

// ---- 轮询 ----

const POLL_MS = 1000;
const ENGINE_EVERY = 3;
const ERROR_FETCH_PER_TICK = 4;

let pollGeneration = 0;

/** 配对成功后开始, 返回停止函数。每一轮等上一轮结束再排下一轮, 不会叠请求。 */
export function startPolling(): () => void {
  const generation = ++pollGeneration;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let tick = 0;
  let saved: readonly SavedJob[] | undefined;
  let jobsRestored = false;
  const alive = () => generation === pollGeneration;

  const run = async () => {
    const client = api();
    if (!alive() || !client) return;
    try {
      // 恢复分两步: 本地文件不需要 Daemon, 先放回来; Job 要问 Daemon, 首轮请求失败时下一轮再试, 不然刷新页面后遇上网络抖动列表就丢了。
      if (saved === undefined) {
        saved = await readSaved();
        await restoreLocals(saved);
      }
      if (!jobsRestored) {
        await restoreJobs(client, saved);
        jobsRestored = true;
      }
      commit(applyJobList(board, await client.jobs()));
      for (const id of jobsNeedingError(board).slice(0, ERROR_FETCH_PER_TICK)) {
        const detail = await client.job(id);
        commit(applyJobEvent(board, id, { type: "errorFetched", error: detail.error ?? "" }).change);
      }
      if (notice() !== null && notice()!.startsWith("无法连接")) notice.set(null);
    } catch (error) {
      if (!handleError(error) && error instanceof DaemonError && error.kind === "network") {
        notice.set(error.message);
      }
    }
    if (tick++ % ENGINE_EVERY === 0) await refreshEngine();
    pump();
    if (alive()) timer = setTimeout(run, POLL_MS);
  };

  void run();
  return () => {
    pollGeneration++;
    if (timer !== undefined) clearTimeout(timer);
  };
}

/** 宿主凭 fileId 还留着的待上传文件。老宿主没有 files.resolve (调用失败) 就当一个都没有。 */
async function hostedFiles(saved: readonly SavedJob[]): Promise<Map<string, LocalFile>> {
  const ids = saved.flatMap((row) => fileIdOfKey(row.id) ?? []);
  try {
    return new Map((await hostPort.resolve(ids)).map((file) => [file.id, file] as const));
  } catch {
    return new Map();
  }
}

/** 这台 Daemon 的存档; 读不出来就当没有。 */
async function readSaved(): Promise<SavedJob[]> {
  const s = session();
  if (s.kind !== "paired") return [];
  try {
    const raw = await Storage.getItem(storageKey(s.daemon.url));
    return raw ? decodeSaved(raw) : [];
  } catch {
    return [];
  }
}

/**
 * 把恢复出来的条目放回列表。存档里的顺序 (savedKeys) 管所有它认得的条目, 包括之前已经放回来的; 这一次会话里新加的、存档里没有的追加在后面。
 * 已经在列表里的条目不动: 重新配对后轮询重启, 正在上传的文件不能被存档里的旧状态盖掉。
 */
function restoreInto(restored: Board, savedKeys: readonly string[]) {
  const items: Record<string, Item> = { ...restored.items };
  for (const key of board.submitOrder) items[key] = board.items[key]!;
  const arrange = (current: readonly string[]): string[] => {
    const placed = new Set<string>();
    const known = savedKeys.filter((key) => items[key] !== undefined && !placed.has(key) && placed.add(key));
    return [...known, ...current.filter((key) => !placed.has(key))];
  };
  const merged: Board = { items, submitOrder: arrange(board.submitOrder), displayOrder: arrange(board.displayOrder) };
  commit({ board: merged, changed: Object.keys(restored.items), orderChanged: true });
}

/**
 * 还没从 Daemon 恢复的 Job 行。本地文件先于 Job 放回列表 (它们不需要 Daemon), 放回之后的那次保存只含已经在列表里的条目,
 * 所以这些行要一起写回去, 不然 Daemon 暂时连不上的时候, 存档里的 Job 就被覆盖掉了。
 */
let unrestoredJobs: readonly SavedJob[] = [];

/**
 * 第一步, 不需要 Daemon: 宿主凭存档里的 fileId 找回还没上传的文件, 放回列表 (上传在 Daemon 可用之后接着做)。
 * 找不回的 (宿主被系统清理了、网页刷新了) 说一声, 不悄悄丢。
 */
async function restoreLocals(saved: readonly SavedJob[]): Promise<void> {
  const isLocal = (row: SavedJob) => fileIdOfKey(row.id) !== undefined;
  unrestoredJobs = saved.filter((row) => !isLocal(row) && board.items[row.id] === undefined);
  const rows = saved.filter((row) => isLocal(row) && board.items[row.id] === undefined);
  if (rows.length === 0) return;
  const { board: restored, lost } = restoreBoard(rows, [], await hostedFiles(rows));
  if (lost > 0) showToast(`有 ${lost} 张还没上传的图片找不到了 (文件已被清理, 网页版刷新页面也会丢), 请重新添加`, "error", { ms: LONG_MS });
  restoreInto(restored, saved.map((row) => row.id));
}

/** 第二步: 从保存的 id 恢复这台 Daemon 上自己提交过的 Job, Daemon 已不认识的丢弃。 */
async function restoreJobs(client: NonNullable<ReturnType<typeof api>>, saved: readonly SavedJob[]): Promise<void> {
  const rows = saved.filter((row) => fileIdOfKey(row.id) === undefined && board.items[row.id] === undefined);
  const entries = await client.jobs();
  if (rows.length > 0) restoreInto(restoreBoard(rows, entries, new Map()).board, saved.map((row) => row.id));
  unrestoredJobs = [];
}

/** 更换 Daemon: 列表属于旧 Daemon, 清空, 并让宿主释放还没上传的文件。 */
export function resetBoard() {
  const abandoned = board.submitOrder.flatMap((key) => {
    const item = board.items[key];
    return item?.kind === "pending" || item?.kind === "uploading" ? [item.file.id] : [];
  });
  hostPort.release(abandoned);
  board = emptyBoard;
  unrestoredJobs = [];
  for (const fileId of abandoned) emitOutcome({ type: "removed", fileId });
  itemSignals.clear();
  batch(() => {
    displayOrder.set([]);
    highlight.set(null);
    counts.set(countItems(board));
    notice.set(null);
  });
}