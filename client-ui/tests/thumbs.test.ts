import { describe, expect, test } from "nsh/test";
import type { Item } from "../src/domain/board";
import { thumbUrl } from "../src/pages/JobRow";

const file = (thumb?: string) => ({ id: "f1", name: "f1.png", size: 10, type: "image/png", ...(thumb === undefined ? {} : { thumb }) });
const THUMB = "data:image/jpeg;base64,/9j/4AAQ";

describe("the picture shown for an item", () => {
  test("a file that is not a Job yet shows the thumbnail the host made, whether it waits, uploads or was refused", () => {
    const items: Item[] = [
      { kind: "pending", key: "local:f1", file: file(THUMB) },
      { kind: "uploading", key: "local:f1", file: file(THUMB) },
      { kind: "pending", key: "local:f1", file: file(THUMB), rejected: "格式不支持" },
    ];
    for (const item of items) expect(thumbUrl(item)).toBe(THUMB);
  });

  test("without a thumbnail the row keeps its placeholder icon instead of requesting anything", () => {
    expect(thumbUrl({ kind: "pending", key: "local:f1", file: file() })).toBe("");
  });
});
