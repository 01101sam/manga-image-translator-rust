import { describe, expect, test } from "nsh/test";
import {
  canPressSpace,
  clampPage,
  newPage,
  nextRender,
  pageStatus,
  reducePage,
  renderLongEdge,
  RENDER_AHEAD,
  spaceLabel,
  translatedJobId,
  translateAllOrder,
  untranslatedIndexes,
  DEFAULT_VIEWING,
  flipViewing,
  followHost,
  parseViewing,
  type PageEvent,
  type ReaderPage,
} from "../src/domain/reader";

const page = (index: number, translation: ReaderPage["translation"], extra: Partial<ReaderPage> = {}): ReaderPage => ({
  index,
  translation,
  ...extra,
});

const run = (start: ReaderPage, ...events: PageEvent[]) => events.reduce((p, event) => reducePage(p, event)[0], start);

// 前六个用例逐个移植自旧的 app/ImageTranslatorTests/PageMachineTests.swift (已删除, 见 git 历史)。
describe("PageMachine (ported from the Swift tests)", () => {
  test("space on an untranslated page enqueues and leaves the state", () => {
    const p = newPage(0);
    expect(reducePage(p, { type: "space" })).toEqual([p, "enqueue"]);
  });

  test("submitted, then running, then done", () => {
    let p = newPage(1);
    p = run(p, { type: "submitted", jobId: "job-1" });
    expect(p).toEqual(page(1, "queued", { jobId: "job-1" }));
    p = run(p, { type: "jobState", state: "running" });
    expect(p.translation).toBe("running");
    p = run(p, { type: "jobState", state: "done" });
    expect(p).toEqual(page(1, "done", { jobId: "job-1" }));
  });

  test("space on a done page asks to flip the document's view and leaves the page alone", () => {
    const p = page(2, "done", { jobId: "job-2" });
    expect(reducePage(p, { type: "space" })).toEqual([p, "toggleView"]);
  });

  test("space on a queued or running page does nothing", () => {
    for (const translation of ["queued", "running"] as const) {
      const p = page(0, translation, { jobId: "j" });
      expect(reducePage(p, { type: "space" })).toEqual([p, "none"]);
    }
  });

  test("failed and cancelled Jobs send the page back to untranslated", () => {
    const p = page(3, "running", { jobId: "j3" });
    expect(run(p, { type: "jobState", state: "failed" })).toEqual(page(3, "untranslated"));
    expect(run(p, { type: "jobState", state: "cancelled" })).toEqual(page(3, "untranslated"));
  });

  test("translate all selects only the untranslated indexes", () => {
    const pages = [page(0, "done", { jobId: "a" }), page(1, "untranslated"), page(2, "queued", { jobId: "b" }), page(3, "untranslated")];
    expect(untranslatedIndexes(pages)).toEqual([1, 3]);
  });
});

describe("PageMachine (what the TSX reader adds)", () => {
  test("an accepted page is preparing, and a second space does not enqueue it again", () => {
    const [accepted] = reducePage(newPage(4), { type: "enqueued" });
    expect(accepted.translation).toBe("preparing");
    expect(reducePage(accepted, { type: "space" })).toEqual([accepted, "none"]);
    expect(canPressSpace(accepted)).toBe(false);
    expect(spaceLabel(accepted, "translated")).toBe("翻译中…");
  });

  test("enqueued only moves an untranslated page", () => {
    const queued = page(0, "queued", { jobId: "j" });
    expect(reducePage(queued, { type: "enqueued" })[0]).toBe(queued);
  });

  test("a failed render or a rejected upload returns a preparing page, and only a preparing page", () => {
    const preparing = page(5, "preparing");
    expect(run(preparing, { type: "prepareFailed" })).toEqual(newPage(5));
    const queued = page(5, "queued", { jobId: "j" });
    expect(reducePage(queued, { type: "prepareFailed" })[0]).toBe(queued);
  });

  test("a lost Artifact resets a done page so it can be translated again", () => {
    const done = page(6, "done", { jobId: "j" });
    expect(run(done, { type: "artifactLost" })).toEqual(newPage(6));
    const running = page(6, "running", { jobId: "j" });
    expect(reducePage(running, { type: "artifactLost" })[0]).toBe(running);
  });

  test("the translation covers the page only when it is done and the document's view is translated", () => {
    expect(translatedJobId(page(0, "done", { jobId: "j" }), "translated")).toBe("j");
    expect(translatedJobId(page(0, "done", { jobId: "j" }), "original")).toBeNull();
    expect(translatedJobId(page(0, "running", { jobId: "j" }), "translated")).toBeNull();
  });

  test("a document starts on the translation, flipping is symmetric, and a stored preference that is not one of the two is ignored", () => {
    expect(DEFAULT_VIEWING).toBe("translated");
    expect(flipViewing("translated")).toBe("original");
    expect(flipViewing(flipViewing("translated"))).toBe("translated");
    expect(parseViewing("original")).toBe("original");
    for (const junk of [undefined, null, 3, "orig", ""]) expect(parseViewing(junk)).toBe("translated");
  });

  test("the space button says what the key would do", () => {
    expect(spaceLabel(newPage(0), "translated")).toBe("翻译本页");
    expect(spaceLabel(page(0, "done", { jobId: "j" }), "original")).toBe("看译文");
    expect(spaceLabel(page(0, "done", { jobId: "j" }), "translated")).toBe("看原文");
    expect(canPressSpace(page(0, "queued", { jobId: "j" }))).toBe(false);
  });
});

describe("reader status text (the old UI tests match on it)", () => {
  test("the page count comes first and the state after it", () => {
    expect(pageStatus(newPage(0), 3, "translated")).toBe("第 1/3 页 · 未翻译");
    expect(pageStatus(page(1, "preparing"), 3, "translated")).toBe("第 2/3 页 · 准备中");
    expect(pageStatus(page(1, "queued", { jobId: "j" }), 3, "translated")).toBe("第 2/3 页 · 排队中");
    expect(pageStatus(page(1, "running", { jobId: "j" }), 3, "translated")).toBe("第 2/3 页 · 翻译中");
    expect(pageStatus(page(2, "done", { jobId: "j" }), 3, "original")).toBe("第 3/3 页 · 已翻译 · 原文");
    expect(pageStatus(page(2, "done", { jobId: "j" }), 3, "translated")).toBe("第 3/3 页 · 已翻译 · 译文");
  });
});

describe("reader navigation and planning", () => {
  test("pages clamp to the document", () => {
    expect(clampPage(-3, 10)).toBe(0);
    expect(clampPage(99, 10)).toBe(9);
    expect(clampPage(4.7, 10)).toBe(4);
    expect(clampPage(2, 0)).toBe(0);
  });

  test("translate all starts at the current page, wraps around and skips what is already translated", () => {
    const pages = [0, 1, 2, 3, 4, 5].map((i) => (i === 3 ? page(i, "done", { jobId: "j" }) : newPage(i)));
    expect(translateAllOrder(pages, 2)).toEqual([2, 4, 5, 0, 1]);
    expect(translateAllOrder(pages, 0)).toEqual([0, 1, 2, 4, 5]);
    expect(translateAllOrder(pages, 5)).toEqual([5, 0, 1, 2, 4]);
  });

  test("pages render one at a time and stop once enough rendered pages wait for their upload", () => {
    expect(nextRender([7, 8], false, 0)).toBe(7);
    expect(nextRender([7, 8], true, 0)).toBeNull();
    expect(nextRender([7, 8], false, RENDER_AHEAD - 1)).toBe(7);
    expect(nextRender([7, 8], false, RENDER_AHEAD)).toBeNull();
    expect(nextRender([], false, 0)).toBeNull();
  });

  test("the page image follows the Daemon's downscale_long_edge, within what a host can render", () => {
    expect(renderLongEdge({ downscale_long_edge: 4096 })).toBe(4096);
    expect(renderLongEdge({ downscale_long_edge: null })).toBe(3072);
    expect(renderLongEdge({})).toBe(3072);
    expect(renderLongEdge(null)).toBe(3072);
    expect(renderLongEdge({ downscale_long_edge: 0 })).toBe(3072);
    expect(renderLongEdge({ downscale_long_edge: 100 })).toBe(512);
    expect(renderLongEdge({ downscale_long_edge: 50_000 })).toBe(8192);
  });

  // 宿主 (PDFView) 对每次换页都发 pageChanged; 只在两边本来同步时才跟着走。
  const feed = (current: number, reports: number[], synced: number | null = null) => {
    let page = current;
    let host = synced;
    for (const reported of reports) {
      const next = followHost(host, page, 100, reported);
      host = reported;
      if (next !== null) page = next;
    }
    return page;
  };

  test("remounting at page 7: the host reports 0, then 7, then 7, and the page stays 7", () => {
    expect(feed(7, [0, 7, 7])).toBe(7);
    expect(feed(7, [0, 7])).toBe(7);
  });

  test("holding an arrow key from 9 to 12: the late echoes 10, 11, 12 never pull the page back", () => {
    // TSX 已经在 12, 宿主上一次报告的是 9
    expect(feed(12, [10, 11, 12], 9)).toBe(12);
    expect(feed(12, [10, 12], 9)).toBe(12);
  });

  test("a swipe or a PDF link after the host caught up is the user's own page", () => {
    expect(feed(12, [13], 12)).toBe(13);
    expect(feed(12, [10, 11, 12, 13], 9)).toBe(13);
    expect(feed(12, [40], 12)).toBe(40);
    expect(feed(12, [13, 14], 12)).toBe(14);
  });

  test("a report for a page that does not exist is ignored", () => {
    expect(followHost(5, 5, 100, 100)).toBeNull();
    expect(followHost(5, 5, 100, -1)).toBeNull();
    expect(followHost(5, 5, 100, 5.5)).toBeNull();
  });
});