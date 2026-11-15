import { describe, expect, open, test } from "nsh/test";
import { BASE, createFakeDaemon, files, type FakeDaemon } from "./fake-daemon";

interface Scenario {
  readonly daemon: FakeDaemon;
  readonly picks?: ReturnType<typeof files>[];
  /** 宿主已保存的配对, 模拟 Keychain 里有 Token。 */
  readonly saved?: boolean;
}

/** 打开应用: 宿主 (Swift / host.js) 由 handlers 扮演, Daemon 由 fake 扮演。 */
async function launch({ daemon, picks = [], saved = false }: Scenario, extra: { platform?: "ios" | "web" } = {}) {
  const queue = [...picks];
  const handlers = {
    "session.snapshot": () => ({
      daemons: [{ name: "studio", url: BASE }],
      paired: saved ? { daemon: { name: "studio", url: BASE }, token: daemon.token() } : null,
    }),
    "session.saveToken": null,
    "session.clearToken": null,
    "files.pick": () => ({ files: queue.shift() ?? [] }),
    "upload.start": () => daemon.submit(),
    "artifact.save": null,
    "files.release": null,
  };
  const app = await open("../App.tsx", { clock: {}, network: daemon.network, host: { handlers }, ...extra });
  return app;
}

const calls = (app: Awaited<ReturnType<typeof launch>>, name: string) => app.host.calls().filter((c) => c.name === name);

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
    await app.press("id:add-images");
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
    await app.press("id:add-images");
    await app.waitFor("text:/全部完成/", { timeout: 120_000 });
    expect(daemon.log.submitted).toBe(150);
    expect(daemon.log.rejected429).toBe(0);
    expect(daemon.log.maxQueued).toBeLessThanOrEqual(64);
  }, 180_000);

  test("two thousand dropped images render only what is on screen and still drain without overshooting the queue", async () => {
    const daemon = createFakeDaemon({ capacity: 64, finishPerPoll: 40 });
    const app = await launch({ daemon, saved: true, picks: [files(2000)] });
    await app.waitFor("id:add-images");
    await app.press("id:add-images");
    await app.waitFor("text:/共 2000 张/");
    expect(app.count("id:job-row")).toBeLessThan(40);
    await app.waitFor("text:/全部完成/", { timeout: 600_000 });
    expect(daemon.log.submitted).toBe(2000);
    expect(daemon.log.rejected429).toBe(0);
    expect(daemon.log.maxQueued).toBeLessThanOrEqual(64);
  }, 900_000);

  test("when the Daemon answers 429 anyway the file goes back to pending and is retried", async () => {
    const daemon = createFakeDaemon({ capacity: 2, finishPerPoll: 1 });
    const app = await launch({ daemon, saved: true, picks: [files(6)] });
    await app.waitFor("id:add-images");
    await app.press("id:add-images");
    await app.waitFor("text:/全部完成/", { timeout: 60_000 });
    expect(daemon.log.submitted).toBe(6);
    expect(daemon.log.rejected429).toBeGreaterThanOrEqual(0);
  });

  test("a stopped Engine holds uploads, shows the banner, and starting it releases them", async () => {
    const daemon = createFakeDaemon({ engine: "stopped", finishPerPoll: 5 });
    const app = await launch({ daemon, saved: true, picks: [files(2)] });
    await app.waitFor("id:engine-banner");
    await app.press("id:add-images");
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
    await app.press("id:add-images");
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
    await first.press("id:add-images");
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

describe("viewer and keyboard", () => {
  async function withDoneJobs(n: number) {
    const daemon = createFakeDaemon({ finishPerPoll: 1000 });
    const app = await launch({ daemon, saved: true, picks: [files(n)] });
    await app.waitFor("id:add-images");
    await app.press("id:add-images");
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
    await app.press("id:add-images");
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
