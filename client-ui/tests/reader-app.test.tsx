import { describe, expect, open, test } from "nsh/test";
import { BASE, createFakeDaemon, type FakeDaemon } from "./fake-daemon";

const pdfFile = (pageCount: number | undefined = 5) => ({ id: "pdf-1", name: "vol01.pdf", size: 5000, type: "application/pdf", pageCount });

interface Scenario {
  readonly daemon: FakeDaemon;
  readonly pageCount?: number | null;
  readonly platform?: "ios" | "web";
  /** 起始分区: 默认直接打开阅读器。 */
  readonly section?: "jobs" | "reader";
  readonly renderFails?: boolean;
  readonly uploadOutcome?: () => unknown;
}

/** 宿主 (Swift / host.js) 由 handlers 扮演, Daemon 由 fake 扮演。 */
async function launch(scenario: Scenario) {
  const { daemon } = scenario;
  const rendered: { fileId: string; page: number; longEdge: number }[] = [];
  const uploaded: string[] = [];
  const shown: { fileId: string; page: number; hidden: boolean }[] = [];
  const handlers = {
    "session.snapshot": () => ({
      daemons: [{ name: "studio", url: BASE }],
      paired: { daemon: { name: "studio", url: BASE }, token: daemon.token() },
    }),
    "session.saveToken": null,
    "session.clearToken": null,
    "files.pick": () => ({ files: [] }),
    "files.release": null,
    "artifact.save": null,
    "upload.start": (data: { fileId: string }) => {
      uploaded.push(data.fileId);
      return scenario.uploadOutcome ? scenario.uploadOutcome() : daemon.submit();
    },
    "pdf.pick": () => ({ file: pdfFile(scenario.pageCount === null ? undefined : (scenario.pageCount ?? 5)) }),
    "pdf.release": null,
    "pdf.renderPage": (data: { fileId: string; page: number; longEdge: number }) => {
      if (scenario.renderFails) throw new Error("内存不足");
      rendered.push(data);
      return { fileRef: { id: "render-" + data.page + "-" + rendered.length, name: "vol01-p" + (data.page + 1) + ".png", size: 4096, type: "image/png" } };
    },
    "pdf.show": (data: { fileId: string; page: number; hidden: boolean }) => {
      shown.push(data);
      return { pageCount: scenario.pageCount ?? 5 };
    },
    "pdf.dispose": null,
  };
  const app = await open("../App.tsx", {
    clock: {},
    network: daemon.network,
    platform: scenario.platform ?? "web",
    host: { params: { section: scenario.section ?? "reader" }, handlers },
  });
  return { app, rendered, uploaded, shown };
}

type Launched = Awaited<ReturnType<typeof launch>>["app"];

const calls = (app: Launched, name: string) => app.host.calls().filter((c) => c.name === name);
const posts = (app: Launched, name: string) => app.host.posts().filter((p) => p.name === name);

async function openPdf(app: Launched) {
  await app.waitFor("id:reader-open");
  await app.press("id:reader-open");
  await app.waitFor("id:reader-status");
}

const status = (app: Launched) => app.texts().find((t) => /^第 \d+\/\d+ 页/.test(t));

describe("PDF reader on the web (pdf.js overlay)", () => {
  test("opening a PDF shows page 1 and asks the host to show it", async () => {
    const { app, shown } = await launch({ daemon: createFakeDaemon() });
    await openPdf(app);
    expect(status(app)).toBe("第 1/5 页 · 未翻译");
    expect(app.texts()).toContain("vol01.pdf");
    await app.waitFor("id:pdf-slot");
    await app.waitUntil(() => shown.length > 0);
    expect(shown[0]).toEqual({ fileId: "pdf-1", page: 0, hidden: false });
  });

  test("arrow keys turn pages, stop at both ends, and every page reaches the host", async () => {
    const { app, shown } = await launch({ daemon: createFakeDaemon() });
    await openPdf(app);
    await app.keyboard.press("right");
    await app.keyboard.press("down");
    expect(status(app)).toBe("第 3/5 页 · 未翻译");
    for (let i = 0; i < 6; i++) await app.keyboard.press("right");
    expect(status(app)).toBe("第 5/5 页 · 未翻译");
    for (let i = 0; i < 9; i++) await app.keyboard.press("up");
    expect(status(app)).toBe("第 1/5 页 · 未翻译");
    await app.keyboard.press("left");
    expect(status(app)).toBe("第 1/5 页 · 未翻译");
    await app.waitUntil(() => shown.at(-1)?.page === 0 && shown.some((s) => s.page === 4));
    expect(new Set(shown.map((s) => s.page))).toEqual(new Set([0, 1, 2, 3, 4]));
  });

  test("the page count can arrive from the display side when the host could not read it", async () => {
    const { app } = await launch({ daemon: createFakeDaemon(), pageCount: null });
    await app.waitFor("id:reader-open");
    await app.press("id:reader-open");
    await app.waitFor("text:/^第 1\\/5 页/");
  });

  test("space translates the page: rendered at the Daemon's long edge, uploaded like an image, then space flips original and translated", async () => {
    const daemon = createFakeDaemon();
    const { app, rendered, uploaded } = await launch({ daemon });
    await openPdf(app);
    await app.keyboard.press("right");
    await app.keyboard.press("space");
    expect(status(app)).toMatch(/准备中|排队中|翻译中/);
    await app.waitFor("text:/已翻译 · 原文/", { timeout: 30_000 });
    expect(rendered).toEqual([{ fileId: "pdf-1", page: 1, longEdge: 4096 }]);
    expect(uploaded).toEqual(["render-1-1"]);
    expect(daemon.log.submitted).toBe(1);
    expect(app.shows("id:reader-translated")).toBe(false);
    await app.keyboard.press("space");
    expect(status(app)).toBe("第 2/5 页 · 已翻译 · 译文");
    expect(app.shows("id:reader-translated")).toBe(true);
    await app.keyboard.press("space");
    expect(status(app)).toBe("第 2/5 页 · 已翻译 · 原文");
    expect(app.shows("id:reader-translated")).toBe(false);
    // 页图是普通 Job, 也出现在任务列表里。
    await app.press("text:任务");
    await app.waitFor("text:/共 1 张/");
  });

  test("space on a page that is already on its way does not render it twice", async () => {
    const { app, rendered } = await launch({ daemon: createFakeDaemon({ finishPerPoll: 0 }) });
    await openPdf(app);
    await app.keyboard.press("space");
    await app.waitFor("text:/排队中/");
    await app.keyboard.press("space");
    await app.keyboard.press("space");
    await app.wait(1500);
    expect(rendered.length).toBe(1);
    expect(app.shows("id:reader-space")).toBe(true);
  });

  test("translate all renders only a few pages ahead while uploads wait, then drains once the Engine starts", async () => {
    const daemon = createFakeDaemon({ engine: "stopped", finishPerPoll: 1000 });
    const { app, rendered } = await launch({ daemon, pageCount: 20 });
    await openPdf(app);
    await app.keyboard.press("right");
    await app.keyboard.press("right");
    await app.press("id:reader-translate-all");
    await app.waitUntil(() => rendered.length >= 5);
    await app.wait(3000);
    expect(rendered.length).toBe(5);
    expect(rendered.map((r) => r.page)).toEqual([2, 3, 4, 5, 6]);
    expect(daemon.log.submitted).toBe(0);
    await app.waitFor("id:engine-banner");
    await app.press("text:启动 Engine");
    await app.waitUntil(() => rendered.length === 20, { timeout: 120_000 });
    expect(rendered.map((r) => r.page).slice(-2)).toEqual([0, 1]);
    await app.waitUntil(() => daemon.log.submitted === 20, { timeout: 60_000 });
    expect(daemon.log.rejected429).toBe(0);
  }, 240_000);

  test("a page the Daemon rejects goes back to untranslated, says why, and can be tried again", async () => {
    const { app, rendered } = await launch({
      daemon: createFakeDaemon(),
      uploadOutcome: () => ({ ok: false, kind: "rejected", message: "图片太大" }),
    });
    await openPdf(app);
    await app.keyboard.press("space");
    await app.waitFor("text:/被 Daemon 拒绝: 图片太大/");
    expect(status(app)).toBe("第 1/5 页 · 未翻译");
    await app.keyboard.press("space");
    await app.waitUntil(() => rendered.length === 2);
  });

  test("a page that cannot be rendered goes back to untranslated with the reason", async () => {
    const { app } = await launch({ daemon: createFakeDaemon(), renderFails: true });
    await openPdf(app);
    await app.keyboard.press("space");
    await app.waitFor("text:/第 1 页渲染失败: 内存不足/");
    expect(status(app)).toBe("第 1/5 页 · 未翻译");
  });

  test("removing a waiting page image from the Jobs list returns the page to untranslated", async () => {
    const daemon = createFakeDaemon({ engine: "stopped" });
    const { app } = await launch({ daemon });
    await openPdf(app);
    await app.keyboard.press("space");
    await app.waitFor("text:/准备中/");
    await app.press("text:任务");
    await app.waitFor("id:job-row");
    await app.press("label:取消");
    await app.press("text:阅读");
    await app.waitFor("text:/第 1\\/5 页 · 未翻译/");
  });

  test("a PDF dropped on the window opens in the reader, whatever the screen", async () => {
    const { app } = await launch({ daemon: createFakeDaemon(), section: "jobs", pageCount: 7 });
    await app.waitFor("id:jobs-summary");
    await app.host.send("pdf.added", [pdfFile(7)]);
    await app.waitFor("id:reader-status");
    expect(status(app)).toBe("第 1/7 页 · 未翻译");
  });

  test("closing the PDF releases it on the host and shows the empty state", async () => {
    const { app } = await launch({ daemon: createFakeDaemon() });
    await openPdf(app);
    await app.press("id:reader-close");
    await app.waitFor("id:reader-empty");
    expect(posts(app, "pdf.release")).toEqual([{ name: "pdf.release", data: { fileId: "pdf-1" } }]);
  });

  test("leaving the reader tears the overlay down and coming back keeps the page and the progress", async () => {
    const { app, shown } = await launch({ daemon: createFakeDaemon() });
    await openPdf(app);
    await app.keyboard.press("right");
    await app.keyboard.press("space");
    await app.waitFor("text:/已翻译/", { timeout: 30_000 });
    const before = shown.length;
    await app.press("text:任务");
    await app.waitFor("id:jobs-summary");
    expect(posts(app, "pdf.dispose").length).toBe(1);
    await app.press("text:阅读");
    await app.waitFor("id:reader-status");
    expect(status(app)).toBe("第 2/5 页 · 已翻译 · 原文");
    await app.waitUntil(() => shown.length > before);
    expect(shown.at(-1)).toEqual({ fileId: "pdf-1", page: 1, hidden: false });
  });
});

describe("PDF reader in the App (native component)", () => {
  test("the PDFView component shows the page, and the web overlay is never driven", async () => {
    const { app, shown } = await launch({ daemon: createFakeDaemon(), platform: "ios" });
    await openPdf(app);
    await app.waitFor("id:pdf-native");
    expect(app.shows("id:pdf-slot")).toBe(false);
    await app.keyboard.press("right");
    expect(status(app)).toBe("第 2/5 页 · 未翻译");
    await app.press("text:任务");
    expect(shown.length).toBe(0);
    expect(calls(app, "pdf.show").length).toBe(0);
    expect(posts(app, "pdf.dispose").length).toBe(0);
  });
});
