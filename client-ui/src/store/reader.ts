import { batch, signal, untracked } from "nsh";
import { localKey } from "../domain/board";
import {
  clampPage,
  newPage,
  nextRender,
  reducePage,
  renderLongEdge,
  translateAllOrder,
  followHost,
  type PageCommand,
  type PageEvent,
  type ReaderPage,
} from "../domain/reader";
import type { JobState } from "../domain/types";
import { pdfPort, type PdfFile } from "../ports/pdf";
import { configOriginal } from "./config";
import { addLocalFiles, itemOf, onFileOutcome, removeItems } from "./jobs";
import { section } from "./route";
import { api, hostPort } from "./session";
import { showToast } from "./toast";

/**
 * PDF 阅读器的状态, 放在 store 里: 切到别的分区页面会卸载, 打开的 PDF 和各页进度要留着。
 * 每页一个 ReaderPage (domain/reader.ts 的状态机); 页怎么变成 Job 见 pumpRenders 与 onFileOutcome。
 */
export interface OpenPdf {
  readonly id: string;
  readonly name: string;
}

export const pdf = signal<OpenPdf | null>(null);
/** 页数还不知道 (宿主没给 pageCount, 组件的 loaded 也没来) 时为空。 */
export const pages = signal<readonly ReaderPage[]>([]);
export const current = signal(0);
export const loadError = signal<string | null>(null);
export const opening = signal(false);

/** 换文档、关文档、换 Daemon 时加一; 异步结果带着发起时的值回来, 对不上就丢掉。 */
let epoch = 0;
/** 还在等渲染的页, 按先后。 */
let renderQueue: number[] = [];
let rendering = false;
/** 已渲染成页图、还没变成 Job 的页: 页图的 fileId -> 页。数量受 RENDER_AHEAD 限制。 */
const prepared = new Map<string, { readonly epoch: number; readonly index: number }>();
let loadWatchdog: ReturnType<typeof setTimeout> | undefined;
let longEdgeFromDaemon: number | null = null;
/** 宿主 (PDFView) 上一次报告的页码; 显示端刚挂载、还没报告过时为 null。 */
let hostPage: number | null = null;

/** 宿主 15 秒内没有报出页数, 就当这个 PDF 打不开 (原生组件没有错误事件)。 */
const LOAD_TIMEOUT_MS = 15_000;

const makePages = (count: number): ReaderPage[] => Array.from({ length: count }, (_, index) => newPage(index));

// ---- 打开与关闭 ----

function armWatchdog() {
  if (loadWatchdog !== undefined) clearTimeout(loadWatchdog);
  loadWatchdog = setTimeout(() => {
    if (pdf() !== null && pages().length === 0) loadError.set("没有读到这个 PDF 的页数, 文件可能已损坏或被移走");
  }, LOAD_TIMEOUT_MS);
}

/** 放掉当前文档: 通知宿主、丢掉还没渲染的页、撤回还没上传的页图。已经成为 Job 的页留在任务列表里。 */
function closeDocument() {
  const open = pdf();
  epoch++;
  if (loadWatchdog !== undefined) clearTimeout(loadWatchdog);
  renderQueue = [];
  hostPage = null;
  const orphans = [...prepared.keys()];
  prepared.clear();
  removeItems(orphans.map(localKey));
  if (open !== null) pdfPort.release(open.id);
}

function openDocument(file: PdfFile) {
  closeDocument();
  batch(() => {
    pdf.set({ id: file.id, name: file.name });
    pages.set(file.pageCount === undefined ? [] : makePages(file.pageCount));
    current.set(0);
    loadError.set(null);
  });
  if (file.pageCount === undefined) armWatchdog();
}

export function closePdf() {
  closeDocument();
  batch(() => {
    pdf.set(null);
    pages.set([]);
    current.set(0);
    loadError.set(null);
  });
}

export async function pickPdf(): Promise<void> {
  if (opening()) return;
  opening.set(true);
  try {
    const file = await pdfPort.pick();
    if (file !== null) openDocument(file);
  } catch (error) {
    showToast(`无法打开 PDF: ${error instanceof Error ? error.message : String(error)}`, "error");
  } finally {
    opening.set(false);
  }
}

/** 页数从显示端 (组件的 loaded 事件, Web 是 pdf.show 的返回值) 报上来。 */
export function documentLoaded(pageCount: number) {
  if (pdf() === null || pages().length === pageCount) return;
  if (loadWatchdog !== undefined) clearTimeout(loadWatchdog);
  batch(() => {
    pages.set(makePages(pageCount));
    current.set(clampPage(current(), pageCount));
    loadError.set(null);
  });
}

export function documentFailed(message: string) {
  if (pdf() !== null) loadError.set(message);
}

// ---- 翻页 ----

export function goTo(page: number) {
  const next = clampPage(page, pages().length);
  if (next !== current()) current.set(next);
}

export const step = (delta: number) => goTo(current() + delta);

/** 原生组件刚挂载 (回到阅读器): 它会从第 0 页开始, 再跳到 TSX 要的页, 这些报告都不是用户翻的。 */
export function surfaceAttached() {
  hostPage = null;
}

/** 宿主报告的换页: 只有用户自己在 PDF 里翻的 (滑动、点链接) 才跟着走, 判断见 followHost。 */
export function nativePageChanged(page: number) {
  const next = followHost(hostPage, current(), pages().length, page);
  hostPage = page;
  if (next !== null) current.set(next);
}

// ---- 页状态 ----

function dispatchPage(index: number, event: PageEvent): PageCommand {
  const list = pages();
  const page = list[index];
  if (page === undefined) return "none";
  const [next, command] = reducePage(page, event);
  if (next !== page) {
    const copy = list.slice();
    copy[index] = next;
    pages.set(copy);
  }
  return command;
}

function dispatchMany(indexes: readonly number[], event: PageEvent) {
  const wanted = new Set(indexes);
  pages.update((list) => list.map((page) => (wanted.has(page.index) ? reducePage(page, event)[0] : page)));
}

/** 空格: 未翻译的页加入 Queue, 翻译完的页在原文和译文之间切换, 其余没有反应。 */
export function pressSpace() {
  const index = current();
  if (dispatchPage(index, { type: "space" }) === "enqueue") enqueue([index]);
}

export function translateAll() {
  enqueue(translateAllOrder(pages(), current()));
}

/** 译文已经不在 Daemon 上 (Daemon 重启过): 这一页重新翻译。 */
export function retranslateCurrent() {
  const index = current();
  dispatchPage(index, { type: "artifactLost" });
  enqueue([index]);
}

function enqueue(indexes: readonly number[]) {
  const list = pages();
  const accepted = indexes.filter((index) => list[index]?.translation === "untranslated");
  if (accepted.length === 0) return;
  dispatchMany(accepted, { type: "enqueued" });
  renderQueue.push(...accepted);
  void pumpRenders();
}

/** 把 Board 里的 Job 状态对到页上。在 effect 里调用: 读到的页和 Job 的 signal 就是依赖, 写入不算。 */
export function syncJobStates() {
  const events: [number, JobState][] = [];
  for (const page of pages()) {
    if (page.jobId === undefined || (page.translation !== "queued" && page.translation !== "running")) continue;
    const item = itemOf(page.jobId);
    if (item?.kind === "job" && item.row.state !== page.translation) events.push([page.index, item.row.state]);
  }
  if (events.length === 0) return;
  untracked(() =>
    batch(() => {
      for (const [index, state] of events) dispatchPage(index, { type: "jobState", state });
    }),
  );
}

// ---- 渲染页图并交给上传 ----

/** 页图长边: 配置页读过就用它的, 否则向 Daemon 取一次。 */
async function pageLongEdge(): Promise<number> {
  const known = configOriginal();
  if (known !== null) return renderLongEdge(known);
  if (longEdgeFromDaemon !== null) return longEdgeFromDaemon;
  const client = api();
  if (client === null) return renderLongEdge(null);
  try {
    longEdgeFromDaemon = renderLongEdge(await client.getConfig());
    return longEdgeFromDaemon;
  } catch {
    return renderLongEdge(null);
  }
}

/** 一次渲染一页, 已渲染未上传的页达到上限就停; 页图作为普通本地文件进 Board, 之后与图片完全一样地分批上传。 */
async function pumpRenders(): Promise<void> {
  const open = pdf();
  const index = nextRender(renderQueue, rendering, prepared.size);
  if (open === null || index === null) return;
  renderQueue.shift();
  rendering = true;
  const mine = epoch;
  try {
    const file = await pdfPort.renderPage(open.id, index, await pageLongEdge());
    if (mine !== epoch) {
      hostPort.release([file.id]);
    } else {
      prepared.set(file.id, { epoch: mine, index });
      addLocalFiles([file]);
    }
  } catch (error) {
    if (mine === epoch) {
      dispatchPage(index, { type: "prepareFailed" });
      showToast(`第 ${index + 1} 页渲染失败: ${error instanceof Error ? error.message : String(error)}`, "error");
    }
  } finally {
    rendering = false;
  }
  void pumpRenders();
}

let started = false;

/** 挂宿主事件和 Board 的回调。只调用一次。 */
export function startReader(): void {
  if (started) return;
  started = true;
  pdfPort.onAdded((files) => {
    const [first, ...extra] = files;
    if (first === undefined) return;
    section.set("reader");
    openDocument(first);
    if (extra.length > 0) showToast(`一次只打开一个 PDF, 已忽略另外 ${extra.length} 个`);
  });
  onFileOutcome((outcome) => {
    const entry = prepared.get(outcome.fileId);
    if (entry === undefined) return;
    prepared.delete(outcome.fileId);
    if (entry.epoch === epoch) {
      switch (outcome.type) {
        case "uploaded":
          dispatchPage(entry.index, { type: "submitted", jobId: outcome.jobId });
          break;
        case "rejected":
          dispatchPage(entry.index, { type: "prepareFailed" });
          showToast(`第 ${entry.index + 1} 页被 Daemon 拒绝: ${outcome.message}`, "error");
          break;
        case "removed":
          dispatchPage(entry.index, { type: "prepareFailed" });
          break;
      }
    }
    void pumpRenders();
  });
}

/** 更换 Daemon: 页进度属于旧 Daemon 的 Job, 连文档一起放掉。 */
export function resetReader() {
  closePdf();
  longEdgeFromDaemon = null;
}
