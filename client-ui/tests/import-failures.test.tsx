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
    }
  }, 60_000);

});
