import { describe, expect, test } from "nsh/test";
import { parseFiles } from "../src/ports/host";

describe("file references from the host", () => {
  test("name, size, type and the modification time are read, and the time is optional", () => {
    expect(
      parseFiles([
        { id: "a", name: "a.png", size: 10, type: "image/png", modifiedAt: 1_700_000_000_000 },
        { id: "b", name: "b.png", size: 20, type: "image/png" },
      ]),
    ).toEqual([
      { id: "a", name: "a.png", size: 10, type: "image/png", modifiedAt: 1_700_000_000_000 },
      { id: "b", name: "b.png", size: 20, type: "image/png" },
    ]);
  });

  test("a modification time that is not a positive number is ignored, so the file sorts as undated", () => {
    const refs = parseFiles([
      { id: "a", name: "a", modifiedAt: 0 },
      { id: "b", name: "b", modifiedAt: -5 },
      { id: "c", name: "c", modifiedAt: "yesterday" },
      { id: "d", name: "d", modifiedAt: null },
      { id: "e", name: "e", modifiedAt: Infinity },
    ]);
    expect(refs.length).toBe(5);
    for (const ref of refs) expect("modifiedAt" in ref).toBe(false);
  });

  test("an entry without an id or a name is dropped, and a missing size is 0", () => {
    expect(parseFiles([{ name: "x" }, { id: "y" }, null, "z", { id: "ok", name: "ok" }])).toEqual([{ id: "ok", name: "ok", size: 0, type: "" }]);
    expect(parseFiles("nope")).toEqual([]);
  });
});
