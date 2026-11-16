import { describe, expect, test } from "nsh/test";
import { parseFiles } from "../src/ports/host";
import { sha256Hex } from "../web/host.js";

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

  test("a thumbnail is kept only when it is a small image data URI, because every one lives in the JS heap", () => {
    const small = "data:image/jpeg;base64," + "A".repeat(100);
    const [kept, tooBig, notData, wrongType] = parseFiles([
      { id: "a", name: "a", thumb: small },
      { id: "b", name: "b", thumb: "data:image/jpeg;base64," + "A".repeat(30_000) },
      { id: "c", name: "c", thumb: "https://elsewhere.test/x.jpg" },
      { id: "d", name: "d", thumb: 5 },
    ]);
    expect(kept!.thumb).toBe(small);
    for (const ref of [tooBig!, notData!, wrongType!]) expect("thumb" in ref).toBe(false);
  });

  test("the fingerprint is kept only as 64 lowercase hex characters, and the aspect ratio comes from a positive width and height", () => {
    const hash = "0123456789abcdef".repeat(4);
    const [ok, noSize, badHash, badSize] = parseFiles([
      { id: "a", name: "a", hash, width: 800, height: 1100 },
      { id: "b", name: "b", hash },
      { id: "c", name: "c", hash: hash.toUpperCase(), width: 800, height: 1100 },
      { id: "d", name: "d", hash, width: 0, height: 5 },
    ]);
    expect(ok).toMatchObject({ hash, aspect: 0.7273 });
    expect("aspect" in noSize!).toBe(false);
    expect(noSize!.hash).toBe(hash);
    expect("hash" in badHash!).toBe(false);
    expect("aspect" in badHash!).toBe(false);
    expect("aspect" in badSize!).toBe(false);
  });

  test("an entry without an id or a name is dropped, and a missing size is 0", () => {
    expect(parseFiles([{ name: "x" }, { id: "y" }, null, "z", { id: "ok", name: "ok" }])).toEqual([{ id: "ok", name: "ok", size: 0, type: "" }]);
    expect(parseFiles("nope")).toEqual([]);
  });
});

describe("SHA-256 in the web host (no crypto.subtle on a plain-http page)", () => {
  const bytes = (text: string) => new Uint8Array([...text].map((c) => c.charCodeAt(0)));

  test("it matches the published test vectors, at block boundaries too", () => {
    expect(sha256Hex(bytes(""))).toBe("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    expect(sha256Hex(bytes("abc"))).toBe("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    expect(sha256Hex(bytes("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"))).toBe("248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
    expect(sha256Hex(new Uint8Array(1_000_000).fill(0x61))).toBe("cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    for (const length of [55, 56, 63, 64, 65, 119, 120, 128]) expect(sha256Hex(new Uint8Array(length).fill(0x61)).length).toBe(64);
  });
});