import { describe, expect, test } from "nsh/test";
import { BASE, createFakeDaemon } from "./fake-daemon";
import { hostFiles, launchPaired } from "./harness";

const KEY = "jobs:" + BASE;

describe("files that are not uploaded yet survive a restart", () => {
  test("they are saved with their names, come back when the host still has them, and upload without being added again", async () => {
    const thumb = "data:image/jpeg;base64,/9j/4AAQ";
    const first = await launchPaired({ daemon: createFakeDaemon({ engine: "stopped" }), picks: [hostFiles(3, "p", { thumb })], width: 390, height: 844 });
    await first.waitFor("id:add-images");
    await first.press("id:add-images");
    await first.press("id:import-files");
    await first.waitFor("text:/待上传 3/");
    await first.clock!.runFor(1000);
    const saved = first.storage()[KEY];
    expect(saved).toBeDefined();
    const rows = JSON.parse(saved!) as unknown[][];
    expect(rows.map((row) => row[0])).toEqual(["local:p0", "local:p1", "local:p2"]);
    expect(rows[0]).toEqual(["local:p0", "p0.png", 1000, null, "image/png", null]);
    expect(saved).not.toContain("base64");

    const daemon = createFakeDaemon({ finishPerPoll: 100 });
    const uploaded: string[] = [];
    const second = await launchPaired({
      daemon,
      storage: { [KEY]: saved! },
      resolve: (ids) => hostFiles(3, "p", { thumb }).filter((file) => ids.includes(file.id) && file.id !== "p2"),
      upload: (data) => {
        uploaded.push(data.fileId);
        return daemon.submit();
      },
      width: 390,
      height: 844,
    });
    await second.waitFor("id:jobs-summary");
    await second.waitFor("text:/共 2 张/");
    await second.waitFor("text:/全部完成/", { timeout: 20_000 });
    expect(uploaded).toEqual(["p0", "p1"]);
    expect(second.texts().some((t) => t.includes("有 1 张还没上传的图片找不到了"))).toBe(true);
    expect(second.count("text:已完成")).toBe(2);
  }, 60_000);

  test("with the Daemon unreachable the files still come back at once, the saved Jobs are not overwritten, and they slot back into their saved places later", async () => {
    const daemon = createFakeDaemon({ engine: "stopped" });
    daemon.jobs.set("job-1-aaaa-bbbb", { state: "done" });
    daemon.jobs.set("job-2-aaaa-bbbb", { state: "done" });
    let offline = true;
    const jobsKey = "GET " + BASE + "/jobs";
    const network = {
      ...daemon.network,
      [jobsKey]: (request: Parameters<(typeof daemon.network)[string]>[0]) =>
        offline ? { status: 503, body: { ok: false, error: "down" } } : daemon.network[jobsKey]!(request),
    };
    const saved = JSON.stringify([
      ["job-1-aaaa-bbbb", "a.png", 5, null],
      ["local:p0", "b.png", 1000, null, "image/png", null],
      ["job-2-aaaa-bbbb", "c.png", 7, null],
    ]);
    const app = await launchPaired({ daemon, network, storage: { [KEY]: saved }, resolve: (ids) => hostFiles(1).filter((file) => ids.includes(file.id)), width: 390, height: 1200 });
    await app.waitFor("text:/待上传 1/");
    await app.clock!.runFor(1000);
    const written = JSON.parse(app.storage()[KEY]!) as string[][];
    expect(written.map((row) => row[0]).sort()).toEqual(["job-1-aaaa-bbbb", "job-2-aaaa-bbbb", "local:p0"]);
    offline = false;
    await app.clock!.runFor(3000);
    await app.waitFor("text:/共 3 张/");
    const names = app.texts().filter((text) => /^[abc]\.png$/.test(text));
    expect(names).toEqual(["a.png", "b.png", "c.png"]);
  });

  test("a refused file comes back still refused, and with Retry", async () => {
    const saved = JSON.stringify([["local:p0", "p0.png", 1000, null, "image/png", "格式不支持"]]);
    const app = await launchPaired({
      daemon: createFakeDaemon(),
      storage: { [KEY]: saved },
      resolve: (ids) => hostFiles(1).filter((file) => ids.includes(file.id)),
      width: 390,
      height: 844,
    });
    await app.waitFor("text:格式不支持");
    expect(app.shows("text:重试")).toBe(true);
  });

  test("an old host without files.resolve loses the files with a notice and the page keeps working", async () => {
    const daemon = createFakeDaemon();
    const saved = JSON.stringify([["local:p0", "p0.png", 1000, null, "image/png", null]]);
    const app = await launchPaired({ daemon, storage: { [KEY]: saved }, resolve: () => { throw new Error("no host function files.resolve"); }, width: 390, height: 844 });
    await app.waitFor("id:jobs-summary");
    await app.waitFor("text:/找不到了/");
    expect(app.shows("id:add-images")).toBe(true);
  });
});