import { describe, expect, test } from "nsh/test";
import { createFakeDaemon } from "./fake-daemon";
import { hostFiles, launchPaired } from "./harness";

const REJECT = "无法识别图片: 不是支持的图片格式";

describe("a mixed batch where the Daemon refuses some files", () => {
  test("every file stays on screen with its name and a status, in the grid and in the list", async () => {
    const bad = new Set(["p3", "p9"]);
    for (const size of [{ width: 1024, height: 1800 }, { width: 390, height: 1800 }]) {
      const daemon = createFakeDaemon({ finishPerPoll: 100 });
      const app = await launchPaired({ daemon, picks: [hostFiles(13)], upload: (d) => (bad.has(d.fileId) ? { ok: false, kind: "rejected", message: REJECT } : daemon.submit()), ...size });
      await app.waitFor("id:add-images");
      await app.press("id:add-images");
      await app.press("id:import-files");
      await app.waitFor("text:/共 13 张/");
      await app.waitUntil(() => app.count("text:已完成") === 11, { timeout: 20_000 });
      expect(app.count("id:job-row")).toBe(13);
      for (let i = 0; i < 13; i++) expect(app.shows("label:/^p" + i + ".png, /")).toBe(true);
      expect(app.count("text:格式不支持")).toBe(2);
      expect(app.texts().some((t) => t.includes("无法识别图片"))).toBe(false);
    }
  }, 60_000);

  test("a refused file stays with the host so Retry can upload it again, and removing it lets the host go", async () => {
    const daemon = createFakeDaemon({ finishPerPoll: 100 });
    const attempts: string[] = [];
    const app = await launchPaired({
      daemon,
      picks: [hostFiles(1)],
      upload: (d) => {
        attempts.push(d.fileId);
        return attempts.length === 1 ? { ok: false, kind: "rejected", message: "图片太大" } : daemon.submit();
      },
      width: 390,
      height: 844,
    });
    await app.waitFor("id:add-images");
    await app.press("id:add-images");
    await app.press("id:import-files");
    await app.waitFor("text:图片太大");
    expect(app.host.posts().filter((p) => p.name === "files.release")).toEqual([]);
    await app.press("text:重试");
    await app.waitFor("text:已完成");
    expect(attempts).toEqual(["p0", "p0"]);
  });

  test("a refused file says why in plain words and keeps the Daemon's own message for other reasons", async () => {
    const daemon = createFakeDaemon();
    const app = await launchPaired({
      daemon,
      picks: [hostFiles(2)],
      upload: (d) => ({ ok: false, kind: "rejected", message: d.fileId === "p0" ? REJECT : "图片 25000x25000 太大, 上限是 200MP 且单边不超过 65535 像素" }),
      width: 390,
      height: 844,
    });
    await app.waitFor("id:add-images");
    await app.press("id:add-images");
    await app.press("id:import-files");
    await app.waitFor("text:格式不支持");
    expect(app.texts().some((t) => t.includes("请转换后重新添加"))).toBe(true);
    expect(app.texts().some((t) => t.includes("25000x25000"))).toBe(true);
  });
});