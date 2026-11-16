import { describe, expect, test } from "nsh/test";
import { BASE, createFakeDaemon } from "./fake-daemon";
import { flipBits, hashFor, launchPaired, type HostFileRef, type Launched } from "./harness";

const KEY = "jobs:" + BASE;
const page = (id: string, hash: string, width = 800, height = 1100): HostFileRef => ({ id, name: id + ".png", size: 1000, type: "image/png", hash, width, height });

async function importOnce(app: Launched) {
  await app.press("id:add-images");
  await app.press("id:import-files");
}

const released = (app: Launched) => app.host.posts().filter((p) => p.name === "files.release").flatMap((p) => (p.data as { ids: string[] }).ids);

describe("skipping pictures that are already in the list", () => {
  test("a picture seen again at another size is skipped with a toast, the button adds it anyway, and nothing is released", async () => {
    const first = [page("p0", hashFor(1)), page("p1", hashFor(2))];
    const second = [page("q0", flipBits(hashFor(1), 10), 300, 413), page("q1", hashFor(3)), page("q2", flipBits(hashFor(3), 5))];
    const app = await launchPaired({ daemon: createFakeDaemon({ engine: "stopped" }), picks: [first, second], width: 390, height: 844 });
    await app.waitFor("id:add-images");
    await importOnce(app);
    await app.waitFor("text:/共 2 张/");
    expect(app.shows("id:toast")).toBe(false);
    await importOnce(app);
    await app.waitFor("text:/共 3 张/");
    await app.waitFor("id:toast");
    expect(app.texts()).toContain("已跳过 2 张重复图片");
    expect(app.shows("id:toast-action")).toBe(true);
    expect(released(app)).toEqual([]);
    await app.press("id:toast-action");
    await app.waitFor("text:/共 5 张/");
    expect(app.shows("id:toast")).toBe(false);
    expect(released(app)).toEqual([]);
  });

  test("when nobody presses the button the skipped copies are released after the toast is gone", async () => {
    const app = await launchPaired({
      daemon: createFakeDaemon({ engine: "stopped" }),
      picks: [[page("p0", hashFor(1))], [page("q0", flipBits(hashFor(1), 3)), page("q1", flipBits(hashFor(1), 4))]],
      width: 390,
      height: 844,
    });
    await app.waitFor("id:add-images");
    await importOnce(app);
    await app.waitFor("text:/共 1 张/");
    await importOnce(app);
    await app.waitFor("text:/已跳过 2 张重复图片/");
    await app.clock!.runFor(9000);
    expect(app.shows("id:toast")).toBe(false);
    expect(released(app).sort()).toEqual(["q0", "q1"]);
    expect(app.texts().some((t) => t.includes("共 1 张"))).toBe(true);
  });

  test("a later toast does not lose what an earlier one was holding: the button adds the files of both batches", async () => {
    const app = await launchPaired({
      daemon: createFakeDaemon({ engine: "stopped" }),
      picks: [[page("p0", hashFor(1))], [page("q0", flipBits(hashFor(1), 2))], [page("r0", flipBits(hashFor(1), 4))]],
      width: 390,
      height: 844,
    });
    await app.waitFor("id:add-images");
    for (let i = 0; i < 3; i++) await importOnce(app);
    await app.waitFor("text:/已跳过 2 张重复图片/");
    expect(released(app)).toEqual([]);
    await app.press("id:toast-action");
    await app.waitFor("text:/共 3 张/");
  });

  test("a removed item stops counting, and a refused or already uploaded one still counts", async () => {
    const daemon = createFakeDaemon({ finishPerPoll: 100 });
    const app = await launchPaired({
      daemon,
      picks: [[page("p0", hashFor(1)), page("p1", hashFor(2))], [page("q0", flipBits(hashFor(1), 4))], [page("r1", flipBits(hashFor(2), 4))], [page("s1", flipBits(hashFor(2), 6))]],
      upload: (data) => (data.fileId === "p1" ? { ok: false, kind: "rejected", message: "图片太大" } : daemon.submit()),
      width: 390,
      height: 1200,
    });
    await app.waitFor("id:add-images");
    await importOnce(app);
    await app.waitFor("text:/已完成/");
    await app.waitFor("text:图片太大");
    // 已上传的 (p0) 和被拒绝的 (p1) 都还在列表里, 都算
    await importOnce(app);
    await app.waitFor("text:/已跳过 1 张重复图片/");
    await importOnce(app);
    await app.waitFor("text:/已跳过 2 张重复图片/");
    expect(app.texts().some((t) => t.includes("共 2 张"))).toBe(true);
    // 移除之后不再算
    await app.press("label:/^p1.png, /");
    await app.keyboard.press("backspace");
    await app.waitFor("text:/共 1 张/");
    await importOnce(app);
    await app.waitFor("text:/共 2 张/");
  });

  test("the fingerprint is saved with the item, so a picture is still recognised after a restart", async () => {
    const thumb = "data:image/jpeg;base64,/9j/4AAQ";
    const first = await launchPaired({ daemon: createFakeDaemon({ engine: "stopped" }), picks: [[{ ...page("p0", hashFor(9)), thumb }]], width: 390, height: 844 });
    await first.waitFor("id:add-images");
    await importOnce(first);
    await first.waitFor("text:/待上传 1/");
    await first.clock!.runFor(1000);
    const rows = JSON.parse(first.storage()[KEY]!) as unknown[][];
    expect(rows[0]).toEqual(["local:p0", "p0.png", 1000, null, "image/png", null, hashFor(9), 0.7273]);

    const second = await launchPaired({
      daemon: createFakeDaemon({ engine: "stopped" }),
      storage: { [KEY]: first.storage()[KEY]! },
      resolve: (ids) => ids.map((id) => ({ id, name: id + ".png", size: 1000, type: "image/png" })),
      picks: [[page("q0", flipBits(hashFor(9), 7), 400, 550)]],
      width: 390,
      height: 844,
    });
    await second.waitFor("text:/待上传 1/");
    await importOnce(second);
    await second.waitFor("text:/已跳过 1 张重复图片/");
    expect(second.texts().some((t) => t.includes("待上传 1"))).toBe(true);
  });

  test("files without a fingerprint and different pictures are all added", async () => {
    const app = await launchPaired({
      daemon: createFakeDaemon({ engine: "stopped" }),
      picks: [[page("a", hashFor(1)), { id: "b", name: "b.png", size: 5, type: "image/png" }], [{ id: "c", name: "c.png", size: 5, type: "image/png" }, page("d", hashFor(2))]],
      width: 390,
      height: 844,
    });
    await app.waitFor("id:add-images");
    await importOnce(app);
    await importOnce(app);
    await app.waitFor("text:/共 4 张/");
    expect(app.shows("id:toast")).toBe(false);
  });
});