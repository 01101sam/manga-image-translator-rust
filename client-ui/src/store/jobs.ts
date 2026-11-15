import { batch, signal, Storage, type Signal } from "nsh";
import { DaemonError } from "../api/daemon";
import {
  addFiles,
  applyJobEvent,
  applyJobList,
  clearableKeys,
  countItems,
  emptyBoard,
  jobsNeedingError,
  planUploads,
  QUEUE_CAPACITY,
  removeKeys,
  restoreJobs,
  savedJobs,
  type SavedJob,
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
import { adjacentDone, moveHighlight } from "../domain/nav";
import type { LocalFile } from "../domain/types";
import { engineAcceptsJobs, engineState, refreshEngine } from "./engine";
import { overrides } from "./overrides";
import { api, handleError, hostPort, session } from "./session";
import { showToast } from "./toast";

/**
 * 列表状态。Board 是纯数据, 这里把它的变化翻译成 signal:
 * 每个 key 一个 signal, 轮询只写真正变化的行; `order` 只在增删时变。
 */
let board: Board = emptyBoard;
const itemSignals = new Map<string, Signal<Item>>();

export const order = signal<readonly string[]>([]);
export const counts = signal<Counts>(countItems(board));
/** 键盘高亮的行 (key)。 */
export const highlight = signal<string | null>(null);
/** 列表上方的持续提示 (Engine 停止、连不上 Daemon)。 */
export const notice = signal<string | null>(null);

export const itemOf = (key: string): Item | undefined => itemSignals.get(key)?.();
export const currentBoard = (): Board => board;

const sameCounts = (a: Counts, b: Counts) => (Object.keys(a) as (keyof Counts)[]).every((k) => a[k] === b[k]);

function commit(change: Change) {
  if (change.board === board) return;
  board = change.board;
  batch(() => {
    for (const key of change.changed) {
      const item = board.items[key];
      if (item === undefined) continue;
      const existing = itemSignals.get(key);
      if (existing) existing.set(item);
      else itemSignals.set(key, signal(item));
    }
    if (change.orderChanged) {
      for (const key of itemSignals.keys()) if (board.items[key] === undefined) itemSignals.delete(key);
      order.set(board.order);
      if (highlight() !== null && board.items[highlight()!] === undefined) highlight.set(null);
      schedulePersist();
    }
    const next = countItems(board);
    if (!sameCounts(next, counts())) counts.set(next);
  });
}

// ---- 持久化: 只存 Job 的 id 顺序, 刷新页面后据此从 Daemon 恢复 ----

let persistTimer: ReturnType<typeof setTimeout> | undefined;
const storageKey = (url: string) => `jobs:${url}`;
const MAX_PERSISTED = 2000;

function schedulePersist() {
  if (persistTimer !== undefined) clearTimeout(persistTimer);
  persistTimer = setTimeout(() => {
    const s = session();
    if (s.kind !== "paired") return;
    // 紧凑格式 [[id, name], ...], 两千项也只有几十 KB。
    const rows = savedJobs(board).slice(-MAX_PERSISTED).map((job) => [job.id, job.name ?? null]);
    void Storage.setItem(storageKey(s.daemon.url), JSON.stringify(rows));
  }, 400);
}

// ---- 操作 ----

export function addLocalFiles(files: readonly LocalFile[]) {
  commit(addFiles(board, files));
  pump();
}

export async function pickAndAdd(): Promise<void> {
  try {
    addLocalFiles(await hostPort.pickFiles(200));
  } catch (error) {
    showToast(`无法选择文件: ${error instanceof Error ? error.message : String(error)}`, "error");
  }
}

export function removeItems(keys: readonly string[]) {
  const releasable = keys.flatMap((key) => {
    const item = board.items[key];
    // 上传中的文件宿主还在读, 等它结束 (成功后宿主自己释放, 失败回到 Pending 才能移除)。
    return item?.kind === "pending" ? [item.file.id] : [];
  });
  const removable = keys.filter((key) => board.items[key]?.kind !== "uploading");
  hostPort.release(releasable);
  commit(removeKeys(board, removable));
}

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
  const keys = board.order;
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
  if (!engineAcceptsJobs()) return;
  for (const key of planUploads(board, { capacity: QUEUE_CAPACITY, concurrency: UPLOAD_CONCURRENCY })) {
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
    commit(uploadSucceeded(board, key, outcome.jobId));
  } else {
    switch (outcome.kind) {
      case "rejected":
        commit(uploadRejected(board, key, outcome.message));
        hostPort.release([item.file.id]);
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
  const alive = () => generation === pollGeneration;

  const run = async () => {
    const client = api();
    if (!alive() || !client) return;
    try {
      if (tick === 0) await restore(client);
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

/** 首轮: 从保存的 id 恢复这台 Daemon 上自己提交过的 Job, 当前内存里已有的 Pending 保持在后面。 */
async function restore(client: NonNullable<ReturnType<typeof api>>): Promise<void> {
  const s = session();
  if (s.kind !== "paired") return;
  const raw = await Storage.getItem(storageKey(s.daemon.url));
  if (!raw) return;
  let saved: SavedJob[];
  try {
    saved = (JSON.parse(raw) as unknown[]).flatMap((row): SavedJob[] => {
      if (typeof row === "string") return [{ id: row }];
      if (Array.isArray(row) && typeof row[0] === "string") return [{ id: row[0], name: typeof row[1] === "string" ? row[1] : undefined }];
      return [];
    });
  } catch {
    return;
  }
  const restored = restoreJobs(saved, await client.jobs());
  const kept = board.order.filter((key) => restored.items[key] === undefined);
  const items: Record<string, Item> = { ...restored.items };
  for (const key of kept) items[key] = board.items[key]!;
  const merged: Board = { order: [...restored.order, ...kept], items };
  commit({ board: merged, changed: merged.order, orderChanged: true });
}

/** 更换 Daemon: 列表属于旧 Daemon, 清空, 并让宿主释放还没上传的文件。 */
export function resetBoard() {
  hostPort.release(
    board.order.flatMap((key) => {
      const item = board.items[key];
      return item?.kind === "pending" || item?.kind === "uploading" ? [item.file.id] : [];
    }),
  );
  board = emptyBoard;
  itemSignals.clear();
  batch(() => {
    order.set([]);
    highlight.set(null);
    counts.set(countItems(board));
    notice.set(null);
  });
}
