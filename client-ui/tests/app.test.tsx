import { describe, expect, open, test } from "nsh/test";
import { BASE, createFakeDaemon, files, type FakeDaemon } from "./fake-daemon";

interface Scenario {
  readonly daemon: FakeDaemon;
  readonly picks?: ReturnType<typeof files>[];
  /** 宿主已保存的配对, 模拟 Keychain 里有 Token。 */
  readonly saved?: boolean;
  /** 宿主在 session.snapshot 里声明的导入入口; 缺省表示老宿主没有声明。 */
  readonly imports?: readonly string[];
  /** files.paste 依次交回的剪贴板内容。 */
  readonly pastes?: ReturnType<typeof files>[];
}

/** 打开应用: 宿主 (Swift / host.js) 由 handlers 扮演, Daemon 由 fake 扮演。 */
async function launch({ daemon, picks = [], saved = false, imports, pastes = [] }: Scenario, extra: { platform?: "ios" | "web"; width?: number; height?: number } = {}) {
  const queue = [...picks];
  const clipboard = [...pastes];
  const handlers = {
    "session.snapshot": () => ({
      daemons: [{ name: "studio", url: BASE }],
      paired: saved ? { daemon: { name: "studio", url: BASE }, token: daemon.token() } : null,
      ...(imports ? { imports } : {}),
    }),
    "session.saveToken": null,
    "session.clearToken": null,
    "files.pick": () => ({ files: queue.shift() ?? [] }),
    "files.paste": () => ({ files: clipboard.shift() ?? [] }),
    "upload.start": () => daemon.submit(),
    "artifact.save": null,
    "files.release": null,
    "pdf.pick": () => ({ file: { id: "pdf-1", name: "vol01.pdf", size: 5000, type: "application/pdf", pageCount: 3 } }),
    "pdf.release": null,
    "pdf.show": () => ({ pageCount: 3 }),
    "pdf.dispose": null,
    "pdf.open": () => ({ file: null }),
  };
  const app = await open("../App.tsx", { clock: {}, network: daemon.network, host: { handlers }, ...extra });
  return app;
}

type Launched = Awaited<ReturnType<typeof launch>>;

const calls = (app: Launched, name: string) => app.host.calls().filter((c) => c.name === name);

/** 点「导入」打开菜单, 再点其中一项。 */
async function importFrom(app: Launched, source: "photos" | "files" | "pdf" = "files") {
  await app.press("id:add-images");
  await app.waitFor("id:import-" + source);
  await app.press("id:import-" + source);
}

describe("pairing", () => {
  test("the default key is prefilled and one tap pairs; the Token goes to the host exactly once", async () => {
    const daemon = createFakeDaemon();
    const app = await launch({ daemon });
    await app.waitFor("id:pairing-key");
    expect(app.value("id:pairing-key")).toBe("sam01101");
    await app.press("id:pairing-confirm");
    await app.waitFor("id:jobs-summary");
    expect(daemon.log.pairs).toEqual(["sam01101"]);
    expect(calls(app, "session.saveToken")).toEqual([{ name: "session.saveToken", data: { daemon: { name: "studio", url: BASE }, token: "tok-1" } }]);
  });

  test("a wrong key says so and stays on the key form", async () => {
    const app = await launch({ daemon: createFakeDaemon({ pairingKey: "other-key" }) });
    await app.waitFor("id:pairing-key");
    await app.press("id:pairing-confirm");
    await app.waitFor("id:pair-notice");
    expect(app.texts()).toContain("Pairing Key 不正确");
    expect(app.shows("id:jobs-summary")).toBe(false);
    expect(calls(app, "session.saveToken").length).toBe(0);
  });

  test("a saved Token skips pairing entirely", async () => {
    const daemon = createFakeDaemon();
    const app = await launch({ daemon, saved: true });
    await app.waitFor("id:jobs-summary");
    expect(daemon.log.pairs).toEqual([]);
    expect(app.shows("id:pairing-key")).toBe(false);
  });

  test("a revoked Token returns to pairing, clears the saved Token and keeps the user's queue", async () => {
    const daemon = createFakeDaemon();
    const app = await launch({ daemon, saved: true, picks: [files(2)] });
    await app.waitFor("id:jobs-summary");
    daemon.revokeTokens();
    await app.waitFor("id:pair-notice");
    expect(app.texts().some((t) => t.includes("配对已失效"))).toBe(true);
    expect(calls(app, "session.clearToken").length).toBe(1);
  });

  test("on the web the Token lives in NSH storage, not in the host", async () => {
    const daemon = createFakeDaemon();
    const app = await launch({ daemon }, { platform: "web" });
    await app.waitFor("id:pairing-key");
    await app.press("id:pairing-confirm");
    await app.waitFor("id:jobs-summary");
    expect(app.storage()["session.token:" + BASE]).toBe("tok-1");
    expect(calls(app, "session.saveToken").length).toBe(0);
  });
});

describe("jobs", () => {
  test("three picked images go through uploading, queued and running to done", async () => {
    const daemon = createFakeDaemon({ finishPerPoll: 1 });
    const app = await launch({ daemon, saved: true, picks: [files(3)] });
    await app.waitFor("id:add-images");
    await importFrom(app);
    await app.waitFor("p0.png");
    await app.waitFor("text:/共 3 张/");
    await app.waitFor("text:/全部完成/", { timeout: 30_000 });
    expect(app.count("text:已完成")).toBe(3);
    expect(daemon.log.submitted).toBe(3);
  });

  test("a batch of 150 never puts more than the queue capacity in flight and never meets a 429", async () => {
    const daemon = createFakeDaemon({ capacity: 64, finishPerPoll: 10 });
    const app = await launch({ daemon, saved: true, picks: [files(150)] });
    await app.waitFor("id:add-images");
    await importFrom(app);
    await app.waitFor("text:/全部完成/", { timeout: 120_000 });
    expect(daemon.log.submitted).toBe(150);
    expect(daemon.log.rejected429).toBe(0);
    expect(daemon.log.maxQueued).toBeLessThanOrEqual(64);
  }, 180_000);

  test("two thousand dropped images render only what is on screen and still drain without overshooting the queue", async () => {
    const daemon = createFakeDaemon({ capacity: 64, finishPerPoll: 40 });
    const app = await launch({ daemon, saved: true, picks: [files(2000)] });
    await app.waitFor("id:add-images");
    await importFrom(app);
    await app.waitFor("text:/共 2000 张/");
    expect(app.count("id:job-row")).toBeLessThan(40);
    await app.waitFor("text:/全部完成/", { timeout: 600_000 });
    expect(daemon.log.submitted).toBe(2000);
    expect(daemon.log.rejected429).toBe(0);
    expect(daemon.log.maxQueued).toBeLessThanOrEqual(64);
  }, 900_000);

  test("the in-flight limit is the capacity the Daemon reports, not a built-in number", async () => {
    const daemon = createFakeDaemon({ capacity: 2, finishPerPoll: 1 });
    const app = await launch({ daemon, saved: true, picks: [files(6)] });
    await app.waitFor("id:add-images");
    await importFrom(app);
    await app.waitFor("text:/全部完成/", { timeout: 60_000 });
    expect(daemon.log.submitted).toBe(6);
    expect(daemon.log.rejected429).toBe(0);
    expect(daemon.log.maxQueued).toBeLessThanOrEqual(2);
  });

  test("when the Daemon answers 429 anyway the file goes back to pending and is retried", async () => {
    const daemon = createFakeDaemon({ capacity: 2, reportedCapacity: 64, finishPerPoll: 1 });
    const app = await launch({ daemon, saved: true, picks: [files(6)] });
    await app.waitFor("id:add-images");
    await importFrom(app);
    await app.waitFor("text:/全部完成/", { timeout: 60_000 });
    expect(daemon.log.submitted).toBe(6);
    expect(daemon.log.rejected429).toBeGreaterThan(0);
  });

  test("a stopped Engine holds uploads, shows the banner, and starting it releases them", async () => {
    const daemon = createFakeDaemon({ engine: "stopped", finishPerPoll: 5 });
    const app = await launch({ daemon, saved: true, picks: [files(2)] });
    await app.waitFor("id:engine-banner");
    await importFrom(app);
    await app.waitFor("p0.png");
    await app.wait(3000);
    expect(daemon.log.submitted).toBe(0);
    await app.press("text:启动 Engine");
    await app.waitFor("text:/全部完成/", { timeout: 30_000 });
    expect(daemon.log.engineActions).toEqual(["start"]);
    expect(daemon.log.submitted).toBe(2);
    expect(app.shows("id:engine-banner")).toBe(false);
  });

  test("a queued job can be cancelled and a finished one removed", async () => {
    const daemon = createFakeDaemon({ finishPerPoll: 0 });
    const app = await launch({ daemon, saved: true, picks: [files(1)] });
    await app.waitFor("id:add-images");
    await importFrom(app);
    await app.waitFor("id:job-cancel");
    await app.press("id:job-cancel");
    await app.waitFor("text:已取消");
    expect([...daemon.jobs.values()][0]?.state).toBe("cancelled");
    await app.press("label:移除");
    await app.waitFor("id:job-row", { state: "gone" });
  });

  test("a restart of the page restores the list from the saved ids", async () => {
    const daemon = createFakeDaemon({ finishPerPoll: 1000 });
    const first = await launch({ daemon, saved: true, picks: [files(2)] });
    await first.waitFor("id:add-images");
    await importFrom(first);
    await first.waitFor("text:/全部完成/", { timeout: 30_000 });
    const stored = first.storage();
    const second = await open("../App.tsx", {
      clock: {},
      network: daemon.network,
      storage: stored,
      host: {
        handlers: {
          "session.snapshot": () => ({ daemons: [{ name: "studio", url: BASE }], paired: { daemon: { name: "studio", url: BASE }, token: daemon.token() } }),
        },
      },
    });
    await second.waitFor("text:/共 2 张/", { timeout: 10_000 });
  });
});

describe("import menu", () => {
  const LABELS = ["从相册", "从文件", "选择文件夹", "打开 PDF", "粘贴"];
  const shownLabels = (app: Launched) => app.texts().filter((t) => LABELS.includes(t));

  test("the paste item reads the clipboard through the host, shows the shortcut, and adds what it returns", async () => {
    const daemon = createFakeDaemon({ finishPerPoll: 1000 });
    const app = await launch({ daemon, saved: true, pastes: [files(2)], imports: ["files", "pdf", "paste"] }, { platform: "ios" });
    await app.waitFor("id:add-images");
    await app.press("id:add-images");
    await app.waitFor("id:import-paste");
    expect(app.texts()).toContain("粘贴");
    expect(app.texts().some((t) => /⌘V|Ctrl\+V/.test(t))).toBe(true);
    await app.press("id:import-paste");
    await app.waitFor("p0.png");
    expect(calls(app, "files.paste").length).toBe(1);
    expect(app.shows("id:import-menu")).toBe(false);
  });

  test("Cmd+V pastes from anywhere and brings the Jobs page back when images arrive", async () => {
    const daemon = createFakeDaemon({ finishPerPoll: 1000 });
    const app = await launch({ daemon, saved: true, pastes: [files(2)], imports: ["files", "pdf", "paste"] }, { platform: "ios" });
    await app.waitFor("id:jobs-summary");
    await app.press("id:tab-config");
    await app.waitFor("id:cfg-workers");
    await app.keyboard.press("mod+v");
    await app.waitFor("id:jobs-summary");
    await app.waitFor("text:/共 2 张/");
    expect(calls(app, "files.paste").length).toBe(1);
  });

  test("an empty clipboard adds nothing and stays on the page", async () => {
    const app = await launch({ daemon: createFakeDaemon(), saved: true, imports: ["files", "pdf", "paste"] }, { platform: "ios" });
    await app.waitFor("id:jobs-summary");
    await app.keyboard.press("mod+v");
    await app.waitUntil(() => calls(app, "files.paste").length === 1);
    expect(app.texts()).toContain("还没有任务");
  });

  test("a host that does not declare paste (the web) leaves Cmd+V to the browser: nothing is bound and the menu has no item", async () => {
    const app = await launch({ daemon: createFakeDaemon(), saved: true, pastes: [files(1)], imports: ["files", "folder", "pdf"] }, { platform: "web" });
    await app.waitFor("id:jobs-summary");
    await app.keyboard.press("mod+v");
    await app.wait(300);
    expect(calls(app, "files.paste").length).toBe(0);
    await app.press("id:add-images");
    await app.waitFor("id:import-menu");
    expect(app.shows("id:import-paste")).toBe(false);
  });

  test("the folder item is a files.pick with source folder", async () => {
    const daemon = createFakeDaemon({ finishPerPoll: 1000 });
    const app = await launch({ daemon, saved: true, picks: [files(3)], imports: ["files", "folder", "pdf"] }, { platform: "web" });
    await app.waitFor("id:add-images");
    await app.press("id:add-images");
    await app.waitFor("id:import-folder");
    expect(shownLabels(app)).toEqual(["从文件", "选择文件夹", "打开 PDF"]);
    await app.press("id:import-folder");
    await app.waitFor("p0.png");
    expect(calls(app, "files.pick")).toEqual([{ name: "files.pick", data: { limit: 200, source: "folder" } }]);
  });

  async function openMenu(app: Launched) {
    await app.waitFor("id:add-images");
    await app.press("id:add-images");
    await app.waitFor("id:import-menu");
  }

  test("lists what the host declares, always in the same order", async () => {
    const app = await launch({ daemon: createFakeDaemon(), saved: true, imports: ["paste", "pdf", "folder", "photos", "files"] }, { platform: "ios" });
    await openMenu(app);
    expect(shownLabels(app)).toEqual(LABELS);
  });

  test("a web host without a photo library gets no photo item, and a host that declares nothing gets the safe two", async () => {
    const web = await launch({ daemon: createFakeDaemon(), saved: true, imports: ["files", "pdf"] }, { platform: "web" });
    await openMenu(web);
    expect(shownLabels(web)).toEqual(["从文件", "打开 PDF"]);
    expect(web.shows("id:import-photos")).toBe(false);
    const old = await launch({ daemon: createFakeDaemon(), saved: true });
    await openMenu(old);
    expect(shownLabels(old)).toEqual(["从文件", "打开 PDF"]);
  });

  test("choosing photos asks the host for the photo library, adds the files and has already closed the menu", async () => {
    const daemon = createFakeDaemon({ finishPerPoll: 1000 });
    const app = await launch({ daemon, saved: true, picks: [files(2)], imports: ["photos", "files", "pdf"] });
    await importFrom(app, "photos");
    await app.waitFor("p0.png");
    expect(calls(app, "files.pick")).toEqual([{ name: "files.pick", data: { limit: 200, source: "photos" } }]);
    expect(app.shows("id:import-menu")).toBe(false);
  });

  test("Escape and a tap outside close the menu without asking the host for anything", async () => {
    const app = await launch({ daemon: createFakeDaemon(), saved: true });
    await openMenu(app);
    await app.keyboard.press("escape");
    await app.waitFor("id:import-menu", { state: "gone" });
    await openMenu(app);
    await app.press("id:import-backdrop");
    await app.waitFor("id:import-menu", { state: "gone" });
    expect(calls(app, "files.pick").length).toBe(0);
    expect(calls(app, "pdf.pick").length).toBe(0);
  });

  test("keys belong to the menu while it is open: Backspace does not remove the highlighted job", async () => {
    const daemon = createFakeDaemon({ finishPerPoll: 1000 });
    const app = await launch({ daemon, saved: true, picks: [files(2)] });
    await importFrom(app);
    await app.waitFor("text:/全部完成/", { timeout: 30_000 });
    await app.keyboard.press("down");
    await app.press("id:add-images");
    await app.waitFor("id:import-menu");
    await app.keyboard.press("backspace");
    await app.keyboard.press("escape");
    await app.waitFor("id:import-menu", { state: "gone" });
    expect(app.texts().some((t) => t.includes("共 2 张"))).toBe(true);
  });

  test("the empty state offers the same menu", async () => {
    const app = await launch({ daemon: createFakeDaemon(), saved: true, imports: ["photos", "files", "pdf"] }, { platform: "ios" });
    await app.waitFor("id:empty-import");
    await app.press("id:empty-import");
    await app.waitFor("id:import-menu");
    expect(shownLabels(app)).toEqual(["从相册", "从文件", "打开 PDF"]);
  });

  test("opening a PDF from the menu asks the host for a PDF and shows the reader", async () => {
    const app = await launch({ daemon: createFakeDaemon(), saved: true }, { platform: "web" });
    await importFrom(app, "pdf");
    await app.waitFor("id:reader-status");
    expect(calls(app, "pdf.pick").length).toBe(1);
    expect(calls(app, "files.pick").length).toBe(0);
  });
});

describe("viewer and keyboard", () => {
  async function withDoneJobs(n: number) {
    const daemon = createFakeDaemon({ finishPerPoll: 1000 });
    const app = await launch({ daemon, saved: true, picks: [files(n)] });
    await app.waitFor("id:add-images");
    await importFrom(app);
    await app.waitFor("text:/全部完成/", { timeout: 30_000 });
    return { daemon, app };
  }

  test("arrows move the highlight, Enter opens, arrows page through done jobs, space flips original and translated, Escape closes", async () => {
    const { app } = await withDoneJobs(3);
    await app.keyboard.press("down");
    await app.keyboard.press("down");
    await app.keyboard.press("enter");
    await app.waitFor("id:viewer");
    expect(app.texts()).toContain("2 / 3");
    await app.keyboard.press("right");
    expect(app.texts()).toContain("3 / 3");
    await app.keyboard.press("right");
    expect(app.texts()).toContain("3 / 3");
    await app.keyboard.press("left");
    expect(app.texts()).toContain("2 / 3");
    await app.keyboard.press("space");
    await app.keyboard.press("space");
    await app.keyboard.press("escape");
    await app.waitFor("id:viewer", { state: "gone" });
  });

  test("space opens the highlighted job like Enter, and Backspace removes the highlighted one", async () => {
    const { app } = await withDoneJobs(3);
    await app.keyboard.press("down");
    await app.keyboard.press("space");
    await app.waitFor("id:viewer");
    expect(app.texts()).toContain("1 / 3");
    await app.keyboard.press("escape");
    await app.waitFor("id:viewer", { state: "gone" });
    await app.keyboard.press("backspace");
    await app.waitFor("text:/共 2 张/");
    expect(app.shows("p0.png")).toBe(false);
  });

  test("saving asks the host for the full-size file, not the w=2048 preview", async () => {
    const { app } = await withDoneJobs(2);
    await app.keyboard.press("down");
    await app.keyboard.press("enter");
    await app.waitFor("id:viewer");
    await app.press("label:保存");
    await app.keyboard.press("space");
    await app.press("label:保存");
    const saves = calls(app, "artifact.save").map((c) => c.data as { url: string; filename: string });
    expect(saves.length).toBe(2);
    expect(saves[0]!.url).toMatch(/\/jobs\/[^/]+\/artifact\?token=tok/);
    expect(saves[0]!.filename).toBe("p0-translated");
    expect(saves[1]!.url).toMatch(/\/jobs\/[^/]+\/source\?token=tok/);
    expect(saves[1]!.filename).toBe("p0");
    for (const save of saves) expect(save.url).not.toContain("w=");
  });

  test("Backspace leaves a queued job alone: it is still on the Daemon", async () => {
    const daemon = createFakeDaemon({ finishPerPoll: 0 });
    const app = await launch({ daemon, saved: true, picks: [files(2)] });
    await app.waitFor("id:add-images");
    await importFrom(app);
    await app.waitFor("text:/共 2 张/");
    await app.keyboard.press("down");
    await app.keyboard.press("backspace");
    await app.wait(500);
    expect(app.texts().some((t) => t.includes("共 2 张"))).toBe(true);
  });

  test("the viewer loads the translated image at w=2048 with the token, and space switches to the original", async () => {
    const { app } = await withDoneJobs(1);
    const id = "job-1-aaaa-bbbb";
    // 无头运行里图片不真正加载; failImages 让指定 URL 报错, 借此确认查看器请求的就是这个 URL。
    await app.failImages(BASE + "/jobs/" + id + "/source?w=2048&token=tok-1");
    await app.press("p0.png");
    await app.waitFor("id:viewer");
    expect(app.shows("id:viewer-error")).toBe(false);
    await app.keyboard.press("space");
    await app.waitFor("id:viewer-error");
    await app.keyboard.press("space");
    await app.waitFor("id:viewer-error", { state: "gone" });
    await app.failImages(BASE + "/jobs/" + id + "/artifact?w=2048&token=tok-1");
    await app.waitFor("id:viewer-error");
    await app.press("id:viewer-close");
    await app.waitFor("id:viewer", { state: "gone" });
  });
});

describe("config", () => {
  test("edits go back to the Daemon as the whole document with unknown fields intact", async () => {
    const daemon = createFakeDaemon();
    const app = await launch({ daemon, saved: true });
    await app.waitFor("id:jobs-summary");
    await app.press("id:tab-config");
    await app.waitFor("id:cfg-workers");
    expect(app.value("id:cfg-workers")).toBe("2");
    await app.clear("id:cfg-workers");
    await app.type("id:cfg-workers", "4");
    await app.press("id:config-save");
    await app.waitUntil(() => daemon.log.configPuts.length === 1);
    const put = daemon.log.configPuts[0] as { workers: number; translator: { unknown_future_field: number } };
    expect(put.workers).toBe(4);
    expect(put.translator.unknown_future_field).toBe(42);
  });

  test("an invalid value blocks the save and names the field", async () => {
    const daemon = createFakeDaemon();
    const app = await launch({ daemon, saved: true });
    await app.waitFor("id:jobs-summary");
    await app.press("id:tab-config");
    await app.waitFor("id:cfg-workers");
    await app.clear("id:cfg-workers");
    await app.type("id:cfg-workers", "0");
    await app.press("id:config-save");
    await app.waitFor("text:/不能小于 1/");
    expect(daemon.log.configPuts.length).toBe(0);
  });

  test("the API key is masked by default", async () => {
    const app = await launch({ daemon: createFakeDaemon(), saved: true });
    await app.waitFor("id:jobs-summary");
    await app.press("id:tab-config");
    await app.waitFor("id:cfg-api_key");
    expect(app.texts().join("|")).not.toContain("sk-secret");
  });

  test("the Engine card starts, stops and restarts", async () => {
    const daemon = createFakeDaemon();
    const app = await launch({ daemon, saved: true });
    await app.waitFor("id:jobs-summary");
    await app.press("id:tab-config");
    await app.waitFor("id:engine-stop");
    await app.press("id:engine-stop");
    await app.waitFor("text:已停止");
    await app.press("id:engine-start");
    await app.waitFor("text:运行中");
    await app.press("id:engine-restart");
    await app.waitUntil(() => daemon.log.engineActions.length === 3);
    expect(daemon.log.engineActions).toEqual(["stop", "start", "restart"]);
  });

  test("forgetting the Daemon clears the Token and returns to the key form", async () => {
    const app = await launch({ daemon: createFakeDaemon(), saved: true });
    await app.waitFor("id:jobs-summary");
    await app.press("id:tab-config");
    await app.waitFor("id:forget");
    await app.press("id:forget");
    await app.waitFor("id:pairing-key");
    expect(calls(app, "session.clearToken").length).toBe(1);
  });
});
describe("grid on wide screens", () => {
  async function withJobs(n: number, size: { width: number; height: number }) {
    const daemon = createFakeDaemon({ finishPerPoll: 1000 });
    const app = await launch({ daemon, saved: true, picks: [files(n)] }, { platform: "ios", ...size });
    await app.waitFor("id:add-images");
    await importFrom(app);
    await app.waitFor("text:/全部完成/", { timeout: 30_000 });
    return app;
  }

  test("a phone-sized screen shows the list, an iPad-sized one a grid of the same jobs", async () => {
    const phone = await withJobs(6, { width: 390, height: 844 });
    expect(phone.shows("id:job-list")).toBe(true);
    expect(phone.shows("id:job-grid")).toBe(false);
    const ipad = await withJobs(6, { width: 1024, height: 768 });
    expect(ipad.shows("id:job-grid")).toBe(true);
    expect(ipad.shows("id:job-list")).toBe(false);
    expect(ipad.count("id:job-row")).toBe(6);
    expect(ipad.texts().filter((t) => t.startsWith("共 6 张")).length).toBe(1);
  });

  test("rotating a phone to landscape keeps the list, and an iPad turned into a narrow split view drops back to it", async () => {
    const app = await withJobs(4, { width: 1024, height: 768 });
    expect(app.shows("id:job-grid")).toBe(true);
    await app.setViewport(430, 932);
    await app.waitFor("id:job-list");
    await app.setViewport(932, 430);
    await app.wait(300);
    expect(app.shows("id:job-list")).toBe(true);
    await app.setViewport(1024, 768);
    await app.waitFor("id:job-grid");
  });

  test("in the grid the arrows move a tile at a time and a row at a time, and Enter opens the highlighted one", async () => {
    const app = await withJobs(10, { width: 1024, height: 768 });
    await app.keyboard.press("down");
    await app.keyboard.press("right");
    await app.keyboard.press("down");
    await app.keyboard.press("enter");
    await app.waitFor("id:viewer");
    // 4 列: 第一下选第 0 项 (没有高亮时向下选第一个), 右到第 1 项, 再下一行到第 5 项, 查看器里的位置是已完成里的第 6 个。
    await app.waitFor("text:6 / 10");
  });
});
describe("arranging the list", () => {
  const T = 1_700_000_000_000;
  // 名字、大小、日期三种顺序互不相同: 名字 page1 < page2 < page10, 大小 b < c < a, 日期 c < a < b。
  const pages = () => [
    { id: "a", name: "page10.png", size: 3000, type: "image/png", modifiedAt: T + 2000 },
    { id: "b", name: "page2.png", size: 1000, type: "image/png", modifiedAt: T + 3000 },
    { id: "c", name: "Page1.png", size: 2000, type: "image/png", modifiedAt: T + 1000 },
  ];
  const titles = (app: Launched) => app.texts().filter((t) => /^page\d+\.png$/i.test(t));

  /** Engine 停着, 三个文件都留在待上传里, 方便看顺序又不会被 Daemon 改状态。 */
  async function heldPages(extra: { platform?: "ios" | "web"; width?: number; height?: number } = {}) {
    const daemon = createFakeDaemon({ engine: "stopped" });
    const app = await launch({ daemon, saved: true, picks: [pages()] }, extra);
    await app.waitFor("id:add-images");
    await importFrom(app);
    await app.waitFor("text:/共 3 张/");
    return { app, daemon };
  }

  async function sortBy(app: Launched, id: string) {
    await app.press("id:open-sort");
    await app.waitFor("id:sort-menu");
    await app.press("id:sort-" + id);
    await app.waitFor("id:sort-menu", { state: "gone" });
  }

  test("the sort menu offers name, date and size, each ascending and descending, and sorts everything once", async () => {
    const { app } = await heldPages();
    expect(titles(app)).toEqual(["page10.png", "page2.png", "Page1.png"]);
    await app.press("id:open-sort");
    await app.waitFor("id:sort-menu");
    for (const label of ["名称 · 升序", "名称 · 降序", "日期 · 从旧到新", "日期 · 从新到旧", "大小 · 从小到大", "大小 · 从大到小"]) {
      expect(app.shows("label:" + label)).toBe(true);
    }
    await app.press("id:sort-name-asc");
    await app.waitFor("id:sort-menu", { state: "gone" });
    expect(titles(app)).toEqual(["Page1.png", "page2.png", "page10.png"]);
    await sortBy(app, "name-desc");
    expect(titles(app)).toEqual(["page10.png", "page2.png", "Page1.png"]);
    await sortBy(app, "modified-asc");
    expect(titles(app)).toEqual(["Page1.png", "page10.png", "page2.png"]);
    await sortBy(app, "modified-desc");
    expect(titles(app)).toEqual(["page2.png", "page10.png", "Page1.png"]);
    await sortBy(app, "size-asc");
    expect(titles(app)).toEqual(["page2.png", "Page1.png", "page10.png"]);
    await sortBy(app, "size-desc");
    expect(titles(app)).toEqual(["page10.png", "Page1.png", "page2.png"]);
  });

  test("sorting is a one-time rearrangement: files added afterwards go to the end and nothing re-sorts", async () => {
    const daemon = createFakeDaemon({ engine: "stopped" });
    const more = [{ id: "z", name: "page0.png", size: 1, type: "image/png", modifiedAt: T }];
    const app = await launch({ daemon, saved: true, picks: [pages(), more] });
    await importFrom(app);
    await app.waitFor("text:/共 3 张/");
    await sortBy(app, "name-asc");
    await importFrom(app);
    await app.waitFor("text:/共 4 张/");
    expect(titles(app)).toEqual(["Page1.png", "page2.png", "page10.png", "page0.png"]);
  });

  test("sorting does not change the order files are submitted to the Daemon", async () => {
    const { app, daemon } = await heldPages();
    await sortBy(app, "name-asc");
    daemon.setEngine("running");
    await app.waitFor("text:/全部完成/", { timeout: 30_000 });
    expect(calls(app, "upload.start").map((c) => (c.data as { fileId: string }).fileId)).toEqual(["a", "b", "c"]);
  });

  test("the item menu moves one item to the front or the back, and greys out what already is there", async () => {
    const { app } = await heldPages();
    const openMenuOf = async (title: string) => {
      await app.press("label:更多: " + title);
      await app.waitFor("id:item-menu");
    };
    await openMenuOf("Page1.png");
    await app.press("id:move-to-front");
    await app.waitFor("id:item-menu", { state: "gone" });
    expect(titles(app)).toEqual(["Page1.png", "page10.png", "page2.png"]);
    await openMenuOf("Page1.png");
    // 已经在最前: 「移到最前」是置灰的, 按不动, 菜单还开着
    await expect(app.press("id:move-to-front")).rejects.toThrow(/disabled/);
    expect(app.shows("id:item-menu")).toBe(true);
    await app.press("id:move-to-back");
    await app.waitFor("id:item-menu", { state: "gone" });
    expect(titles(app)).toEqual(["page10.png", "page2.png", "Page1.png"]);
    await openMenuOf("page10.png");
    await app.keyboard.press("escape");
    await app.waitFor("id:item-menu", { state: "gone" });
    expect(titles(app)).toEqual(["page10.png", "page2.png", "Page1.png"]);
  });

  test("the arranged order survives a restart: names, sizes and dates come back with the jobs", async () => {
    const daemon = createFakeDaemon({ finishPerPoll: 1000 });
    const first = await launch({ daemon, saved: true, picks: [pages()] });
    await importFrom(first);
    await first.waitFor("text:/全部完成/", { timeout: 30_000 });
    await sortBy(first, "size-desc");
    await first.press("label:更多: page2.png");
    await first.press("id:move-to-front");
    expect(titles(first)).toEqual(["page2.png", "page10.png", "Page1.png"]);
    await first.wait(800);
    const second = await open("../App.tsx", {
      clock: {},
      network: daemon.network,
      storage: first.storage(),
      host: {
        handlers: {
          "session.snapshot": () => ({ daemons: [{ name: "studio", url: BASE }], paired: { daemon: { name: "studio", url: BASE }, token: daemon.token() } }),
        },
      },
    });
    await second.waitFor("text:/共 3 张/", { timeout: 10_000 });
    expect(titles(second)).toEqual(["page2.png", "page10.png", "Page1.png"]);
    // 恢复后大小和日期还在, 再按日期排序能用
    await sortBy(second, "modified-asc");
    expect(titles(second)).toEqual(["Page1.png", "page10.png", "page2.png"]);
  });

  test("sorting needs two items", async () => {
    const daemon = createFakeDaemon({ engine: "stopped" });
    const app = await launch({ daemon, saved: true, picks: [pages().slice(0, 1)] });
    await importFrom(app);
    await app.waitFor("text:/共 1 张/");
    await expect(app.press("id:open-sort")).rejects.toThrow(/disabled/);
    expect(app.shows("id:sort-menu")).toBe(false);
  });
});

describe("drag to reorder", () => {
  const names = (app: Launched) => app.texts().filter((t) => /^p\d+\.png$/.test(t));

  /** Engine 停着, 文件都留在待上传里: 只看顺序。 */
  async function held(n: number, extra: { platform?: "ios" | "web"; width?: number; height?: number } = { platform: "web" }) {
    const daemon = createFakeDaemon({ engine: "stopped" });
    const app = await launch({ daemon, saved: true, picks: [files(n)] }, extra);
    await app.waitFor("id:add-images");
    await importFrom(app);
    await app.waitFor("text:/共 " + n + " 张/");
    return { app, daemon };
  }

  test("with a mouse a row dragged down shows the ghost and the marker, and drops where the marker was", async () => {
    const { app, daemon } = await held(5);
    expect(names(app)).toEqual(["p0.png", "p1.png", "p2.png", "p3.png", "p4.png"]);
    await app.dragAndHold("label:/^p0.png,/", { y: 180 });
    expect(app.shows("id:drag-ghost")).toBe(true);
    expect(app.shows("id:drop-marker")).toBe(true);
    // 拖动中列表原样不动, 顺序要到放手才改
    expect(names(app).filter((t) => t === "p0.png").length).toBe(2);
    await app.drop();
    await app.waitFor("id:drag-ghost", { state: "gone" });
    expect(app.shows("id:drop-marker")).toBe(false);
    expect(names(app)).toEqual(["p1.png", "p2.png", "p3.png", "p0.png", "p4.png"]);
    daemon.setEngine("running");
    await app.waitFor("text:/全部完成/", { timeout: 30_000 });
    expect(calls(app, "upload.start").map((c) => (c.data as { fileId: string }).fileId)).toEqual(["p0", "p1", "p2", "p3", "p4"]);
  });

  test("a row dragged up lands before the row it was dropped on, and a drag that goes nowhere changes nothing", async () => {
    const { app } = await held(5);
    await app.drag("label:/^p4.png,/", { y: -150 });
    await app.waitFor("id:drag-ghost", { state: "gone" });
    expect(names(app)).toEqual(["p0.png", "p1.png", "p4.png", "p2.png", "p3.png"]);
    await app.drag("label:/^p1.png,/", { y: 10 });
    expect(names(app)).toEqual(["p0.png", "p1.png", "p4.png", "p2.png", "p3.png"]);
  });

  test("in the grid a tile dragged to another tile's place shows a vertical marker and lands there", async () => {
    const { app } = await held(8, { platform: "web", width: 1024, height: 1400 });
    await app.waitFor("id:job-grid");
    await app.dragAndHold("label:/^p1.png,/", { x: 440, y: 0 });
    expect(app.shows("id:drag-ghost")).toBe(true);
    expect(app.shows("id:drop-marker")).toBe(true);
    await app.drop();
    await app.waitFor("id:drag-ghost", { state: "gone" });
    const after = names(app);
    expect(after.length).toBe(8);
    expect(after.indexOf("p1.png")).toBeGreaterThan(1);
  });

  test("holding a row at the bottom edge scrolls the list on its own, and the row drops far down", async () => {
    const { app } = await held(60, { platform: "web", width: 420, height: 700 });
    await app.dragAndHold("label:/^p0.png,/", { y: 520 });
    await app.wait(2500);
    await app.drop();
    await app.waitFor("id:drag-ghost", { state: "gone" });
    await app.scroll("id:job-list", -100_000);
    expect(names(app)[0]).toBe("p1.png");
  });

  test("on a touch screen an ordinary swipe scrolls and never reorders, and a drag without a long press does not start", async () => {
    const { app } = await held(40, { platform: "ios", width: 390, height: 844 });
    await app.drag("label:/^p0.png,/", { y: -300 });
    expect(app.shows("id:drag-ghost")).toBe(false);
    expect(app.shows("id:drop-marker")).toBe(false);
    await app.scroll("id:job-list", -100_000);
    expect(names(app).slice(0, 3)).toEqual(["p0.png", "p1.png", "p2.png"]);
  });

  test("after a drop the release does not open the viewer, and pressing a finished job still does", async () => {
    const daemon = createFakeDaemon({ finishPerPoll: 1000 });
    const app = await launch({ daemon, saved: true, picks: [files(4)] }, { platform: "web" });
    await app.waitFor("id:add-images");
    await importFrom(app);
    await app.waitFor("text:/全部完成/", { timeout: 30_000 });
    await app.drag("label:/^p0.png,/", { y: 160 });
    await app.waitFor("id:drag-ghost", { state: "gone" });
    expect(app.shows("id:viewer")).toBe(false);
    expect(names(app)[0]).not.toBe("p0.png");
    // 刚放手的 0.4 秒里点按不算数 (那一下是放手, 不是点按)
    await app.wait(500);
    await app.press("label:/^p0.png,/");
    await app.waitFor("id:viewer");
  });

  test("a row being dragged while its upload finishes stays in the drag and still drops where it was put", async () => {
    const { app, daemon } = await held(5);
    await app.press("label:更多: p1.png");
    await app.waitFor("id:item-menu");
    await app.keyboard.press("escape");
    await app.dragAndHold("label:/^p0.png,/", { y: 180 });
    daemon.setEngine("running");
    await app.wait(6000);
    // 上传成功后 p0 的 key 从本地 id 换成了 jobId: 影子和占位还在, 拖动没有被放弃
    expect(app.shows("id:drag-ghost")).toBe(true);
    expect(app.shows("id:drop-marker")).toBe(true);
    expect(names(app).filter((t) => t === "p0.png").length).toBe(2);
    await app.drop();
    await app.waitFor("id:drag-ghost", { state: "gone" });
    expect(names(app)).toEqual(["p1.png", "p2.png", "p3.png", "p0.png", "p4.png"]);
    expect(calls(app, "upload.start").length).toBe(5);
  });

  test("the item menu of a row stays open and keeps working when that row's upload finishes", async () => {
    const { app, daemon } = await held(3);
    await app.press("label:更多: p2.png");
    await app.waitFor("id:item-menu");
    daemon.setEngine("running");
    await app.wait(6000);
    expect(app.shows("id:item-menu")).toBe(true);
    await app.press("id:move-to-front");
    await app.waitFor("id:item-menu", { state: "gone" });
    expect(names(app)).toEqual(["p2.png", "p0.png", "p1.png"]);
  });
});
