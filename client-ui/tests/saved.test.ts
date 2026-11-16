import { describe, expect, test } from "nsh/test";
import { decodeSaved, encodeSaved } from "../src/domain/saved";

describe("saved jobs", () => {
  test("rows round-trip with name, size and modified time, and missing values stay missing", () => {
    const jobs = [
      { id: "a", name: "page1.png", size: 1234, modifiedAt: 1_700_000_000_000 },
      { id: "b" },
      { id: "c", name: "c.png" },
    ];
    const back = decodeSaved(encodeSaved(jobs));
    expect(back).toEqual([
      { id: "a", name: "page1.png", size: 1234, modifiedAt: 1_700_000_000_000 },
      { id: "b", name: undefined, size: undefined, modifiedAt: undefined },
      { id: "c", name: "c.png", size: undefined, modifiedAt: undefined },
    ]);
  });

  test("the compact form is one array per job in the order given", () => {
    expect(JSON.parse(encodeSaved([{ id: "x", name: "n", size: 5, modifiedAt: 9 }, { id: "y" }]))).toEqual([
      ["x", "n", 5, 9],
      ["y", null, null, null],
    ]);
  });

  test("a file that is not uploaded yet keeps its type and the reason it was refused, and an ordinary row is unchanged", () => {
    const rows = [
      { id: "job-1", name: "a.png", size: 5, modifiedAt: 9 },
      { id: "local:F1", name: "b.jpg", size: 7, modifiedAt: 11, type: "image/jpeg" },
      { id: "local:F2", name: "c.jpg", type: "image/jpeg", rejected: "格式不支持" },
    ];
    expect(JSON.parse(encodeSaved(rows))).toEqual([
      ["job-1", "a.png", 5, 9],
      ["local:F1", "b.jpg", 7, 11, "image/jpeg", null],
      ["local:F2", "c.jpg", null, null, "image/jpeg", "格式不支持"],
    ]);
    expect(decodeSaved(encodeSaved(rows))).toEqual([
      { id: "job-1", name: "a.png", size: 5, modifiedAt: 9 },
      { id: "local:F1", name: "b.jpg", size: 7, modifiedAt: 11, type: "image/jpeg" },
      { id: "local:F2", name: "c.jpg", type: "image/jpeg", rejected: "格式不支持" },
    ]);
  });

  test("the fingerprint and aspect ratio ride along, on Job rows too, and a damaged fingerprint is dropped", () => {
    const hash = "ab".repeat(32);
    const rows = [{ id: "job-1", name: "a.png", size: 5, hash, aspect: 0.7 }, { id: "local:F", name: "b.png", type: "image/png", hash }];
    expect(JSON.parse(encodeSaved(rows))).toEqual([
      ["job-1", "a.png", 5, null, null, null, hash, 0.7],
      ["local:F", "b.png", null, null, "image/png", null, hash, null],
    ]);
    expect(decodeSaved(encodeSaved(rows))).toEqual([
      { id: "job-1", name: "a.png", size: 5, modifiedAt: undefined, hash, aspect: 0.7 },
      { id: "local:F", name: "b.png", size: undefined, modifiedAt: undefined, type: "image/png", hash },
    ]);
    expect(decodeSaved(JSON.stringify([["x", null, null, null, null, null, "not a hash", 0.7]]))).toEqual([{ id: "x", name: undefined, size: undefined, modifiedAt: undefined }]);
  });

  test("older saves still load: bare ids and [id, name] pairs", () => {
    expect(decodeSaved(JSON.stringify(["a", ["b", "b.png"], ["c", null]]))).toEqual([
      { id: "a" },
      { id: "b", name: "b.png", size: undefined, modifiedAt: undefined },
      { id: "c", name: undefined, size: undefined, modifiedAt: undefined },
    ]);
  });

  test("nonsense is dropped row by row, and a broken save is an empty list", () => {
    expect(decodeSaved("not json")).toEqual([]);
    expect(decodeSaved(JSON.stringify({ a: 1 }))).toEqual([]);
    const rows = decodeSaved(JSON.stringify([[1, "x"], [], null, ["ok", 7, "big", "recent"], ["z", "z.png", 0, -3]]));
    expect(rows.map((row) => row.id)).toEqual(["ok", "z"]);
    expect(rows[0]).toEqual({ id: "ok", name: undefined, size: undefined, modifiedAt: undefined });
    // 大小 0 和不合理的时间都当缺失
    expect(rows[1]).toEqual({ id: "z", name: "z.png", size: undefined, modifiedAt: undefined });
  });
});