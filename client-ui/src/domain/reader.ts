import type { JobState } from "./types";

/**
 * PDF 阅读器里一页的翻译进度。比 Daemon 的 JobState 多一个 preparing:
 * 页已加入队列, 但还在渲染成图片或等待上传, 还没有 jobId。
 */
export type Translation = "untranslated" | "preparing" | "queued" | "running" | "done";

export type PageViewing = "original" | "translated";

export interface ReaderPage {
  /** 从 0 起。 */
  readonly index: number;
  readonly translation: Translation;
  readonly viewing: PageViewing;
  readonly jobId?: string;
}

export type PageEvent =
  | { readonly type: "space" }
  /** 阅读器接受了「翻译这一页」, 开始渲染并上传。 */
  | { readonly type: "enqueued" }
  | { readonly type: "submitted"; readonly jobId: string }
  | { readonly type: "jobState"; readonly state: JobState }
  /** 渲染失败, 或页图上传被 Daemon 拒绝、被用户从任务列表移除。 */
  | { readonly type: "prepareFailed" }
  /** 译文图片加载失败 (Daemon 重启后 Artifact 不在了)。 */
  | { readonly type: "artifactLost" };

export type PageCommand = "none" | "enqueue" | "toggleView";

export const newPage = (index: number): ReaderPage => ({ index, translation: "untranslated", viewing: "original" });

const reset = (page: ReaderPage): ReaderPage => newPage(page.index);

function onSpace(page: ReaderPage): [ReaderPage, PageCommand] {
  switch (page.translation) {
    case "untranslated":
      return [page, "enqueue"];
    case "done":
      return [{ ...page, viewing: page.viewing === "original" ? "translated" : "original" }, "toggleView"];
    case "preparing":
    case "queued":
    case "running":
      return [page, "none"];
  }
}

function onJobState(page: ReaderPage, state: JobState): ReaderPage {
  switch (state) {
    case "queued":
      return { ...page, translation: "queued" };
    case "running":
      return { ...page, translation: "running" };
    case "done":
      return { ...page, translation: "done" };
    case "failed":
    case "cancelled":
      return reset(page);
  }
}

/**
 * 从旧 Client 的 reducePage 移植 (app/ImageTranslatorTests/PageMachineTests.swift 的语义不变):
 * 空格在未翻译页上只发出 enqueue 命令, 状态由后续事件推进; 失败和取消回到未翻译。
 */
export function reducePage(page: ReaderPage, event: PageEvent): [ReaderPage, PageCommand] {
  switch (event.type) {
    case "space":
      return onSpace(page);
    case "enqueued":
      return [page.translation === "untranslated" ? { ...page, translation: "preparing" } : page, "none"];
    case "submitted":
      return [{ ...page, translation: "queued", jobId: event.jobId }, "none"];
    case "jobState":
      return [onJobState(page, event.state), "none"];
    case "prepareFailed":
      return [page.translation === "preparing" ? reset(page) : page, "none"];
    case "artifactLost":
      return [page.translation === "done" ? reset(page) : page, "none"];
  }
}

export function untranslatedIndexes(pages: readonly ReaderPage[]): number[] {
  return pages.filter((page) => page.translation === "untranslated").map((page) => page.index);
}

/** 「翻译全部」的顺序: 从当前页往后, 到末尾再回到开头, 用户最先看到的是眼前这几页。 */
export function translateAllOrder(pages: readonly ReaderPage[], current: number): number[] {
  const todo = untranslatedIndexes(pages);
  const from = todo.findIndex((index) => index >= current);
  return from <= 0 ? todo : [...todo.slice(from), ...todo.slice(0, from)];
}

/** 页脚状态文案, 沿用旧 Client 的措辞: UI 测试按「第 N/M 页」和后面的词匹配。 */
export function pageStatus(page: ReaderPage, count: number): string {
  const where = `第 ${page.index + 1}/${count} 页`;
  switch (page.translation) {
    case "untranslated":
      return `${where} · 未翻译`;
    case "preparing":
      return `${where} · 准备中`;
    case "queued":
      return `${where} · 排队中`;
    case "running":
      return `${where} · 翻译中`;
    case "done":
      return `${where} · 已翻译 · ${page.viewing === "translated" ? "译文" : "原文"}`;
  }
}

export const clampPage = (page: number, count: number): number => (count <= 0 ? 0 : Math.min(Math.max(Math.trunc(page), 0), count - 1));

/** 已渲染成图片、还没变成 Job 的页最多这么多; 再多就先不渲染下一页 (每页是几 MB 的 PNG)。 */
export const RENDER_AHEAD = 5;

/** 下一页渲染谁: 同时只渲染一页, 而且上传跟不上 (队列满、Engine 停止) 时不再往前准备。 */
export function nextRender(queue: readonly number[], rendering: boolean, prepared: number): number | null {
  if (rendering || prepared >= RENDER_AHEAD) return null;
  return queue[0] ?? null;
}

const DEFAULT_LONG_EDGE = 3072;
const MIN_LONG_EDGE = 512;
const MAX_LONG_EDGE = 8192;

/** 送去翻译的页图长边: 跟随 Daemon 的 downscale_long_edge (Daemon 反正会缩到这个尺寸), 关闭缩放或没取到时用默认值。 */
export function renderLongEdge(config: Readonly<Record<string, unknown>> | null): number {
  const value = config?.downscale_long_edge;
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) return DEFAULT_LONG_EDGE;
  return Math.min(Math.max(Math.round(value), MIN_LONG_EDGE), MAX_LONG_EDGE);
}

/** 页面现在该盖着哪个 Job 的译文: 已翻译完且选了译文时才有。 */
export const translatedJobId = (page: ReaderPage): string | null => (page.translation === "done" && page.viewing === "translated" ? (page.jobId ?? null) : null);

/** 页脚空格按钮的字: 与键盘空格做同一件事。 */
export function spaceLabel(page: ReaderPage): string {
  switch (page.translation) {
    case "untranslated":
      return "翻译本页";
    case "done":
      return page.viewing === "translated" ? "看原文" : "看译文";
    case "preparing":
    case "queued":
    case "running":
      return "翻译中…";
  }
}

/** 空格按钮在准备、排队、翻译期间按不了。 */
export const canPressSpace = (page: ReaderPage): boolean => page.translation === "untranslated" || page.translation === "done";

/**
 * 宿主报告换页时判断这是不是用户自己翻的。PDFView 对每次换页都发 pageChanged, 包括 TSX 改 page 引起的;
 * 连按方向键时回声滞后, 不能把页码拉回去。`recent` 是 TSX 最近自己要求过的页码; 返回要跳到的页, 回声和越界返回 null。
 */
export function userPage(page: number, current: number, count: number, recent: readonly number[]): number | null {
  if (!Number.isInteger(page) || page < 0 || page >= count || page === current || recent.includes(page)) return null;
  return page;
}
