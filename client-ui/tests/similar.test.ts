import { describe, expect, test } from "nsh/test";
import { addFiles, emptyBoard, fingerprintsOf, removeKeys, startUpload, uploadSucceeded } from "../src/domain/board";
import { ASPECT_TOLERANCE, aspectDifference, distance, fingerprintOf, isSimilar, MIN_BITS, parseHash, SIMILAR_DISTANCE, splitDuplicates } from "../src/domain/similar";
import type { LocalFile } from "../src/domain/types";
import { flipBits, hashFor } from "./harness";

const file = (id: string, hash?: string, aspect?: number): LocalFile => ({
  id,
  name: id + ".png",
  size: 10,
  type: "image/png",
  ...(hash === undefined ? {} : { hash }),
  ...(aspect === undefined ? {} : { aspect }),
});
const print = (hash: string, aspect?: number) => fingerprintOf(hash, aspect)!;

describe("fingerprints", () => {
  test("only 64 lowercase hex characters are a fingerprint", () => {
    expect(parseHash(hashFor(1))?.length).toBe(8);
    for (const bad of [undefined, null, 5, "", "abc", hashFor(1).toUpperCase(), hashFor(1) + "0", "g".repeat(64)]) expect(parseHash(bad)).toBeUndefined();
    expect(fingerprintOf("nope", 1)).toBeUndefined();
  });

  test("the distance counts the differing bits, whatever their position", () => {
    const base = hashFor(7);
    for (const bits of [0, 1, 7, 24, 25, 100, 256]) expect(distance(print(base), print(flipBits(base, bits)))).toBe(bits);
    expect(distance({ words: parseHash("0".repeat(64))! }, print("f".repeat(64)))).toBe(256);
  });

  test("a hash with almost no bits set (a blank or flat picture) is not a fingerprint, because it is near every other such hash", () => {
    const sparse = (bits: number) => flipBits("0".repeat(64), bits);
    expect(fingerprintOf("0".repeat(64), 0.7)).toBeUndefined();
    expect(fingerprintOf(sparse(MIN_BITS), 0.7)).toBeUndefined();
    expect(fingerprintOf(sparse(MIN_BITS + 1), 0.7)).toBeDefined();
    // 画的完全不同, 距离也在阈值以内, 所以认不出才是对的
    const left = "ff80" + "0".repeat(60);
    const right = "0".repeat(60) + "01ff";
    expect(distance({ words: parseHash(left)! }, { words: parseHash(right)! })).toBeLessThanOrEqual(SIMILAR_DISTANCE);
  });

  test("a distance over the limit stops early but is still over the limit", () => {
    const base = hashFor(3);
    expect(distance(print(base), print(flipBits(base, 200)), 10)).toBeGreaterThan(10);
  });

  test("similar means at most the threshold of different bits and an aspect ratio within three percent", () => {
    const base = hashFor(11);
    expect(isSimilar(print(base, 0.7), print(flipBits(base, SIMILAR_DISTANCE), 0.7))).toBe(true);
    expect(isSimilar(print(base, 0.7), print(flipBits(base, SIMILAR_DISTANCE + 1), 0.7))).toBe(false);
    expect(aspectDifference(print(base, 0.7), print(base, 0.72))).toBeLessThan(ASPECT_TOLERANCE);
    expect(isSimilar(print(base, 0.7), print(base, 0.72))).toBe(true);
    expect(isSimilar(print(base, 0.7), print(base, 0.725))).toBe(false);
    expect(isSimilar(print(base), print(base, 2))).toBe(true);
  });

  test("a different picture is far away", () => {
    for (let seed = 1; seed < 40; seed++) expect(isSimilar(print(hashFor(seed), 0.7), print(hashFor(seed + 100), 0.7))).toBe(false);
  });
});

describe("splitting a batch", () => {
  test("a file similar to a known one is skipped, one without a fingerprint never is, and the order is kept", () => {
    const known = [print(hashFor(1), 0.7)];
    const batch = [file("a", flipBits(hashFor(1), 8), 0.7), file("b", hashFor(2), 0.7), file("c"), file("d", hashFor(3), 0.7)];
    const { fresh, skipped } = splitDuplicates(batch, known);
    expect(fresh.map((f) => f.id)).toEqual(["b", "c", "d"]);
    expect(skipped.map((f) => f.id)).toEqual(["a"]);
  });

  test("blank pictures are never skipped, however alike they look", () => {
    const blank = "0".repeat(64);
    const { fresh, skipped } = splitDuplicates([file("w1", blank, 0.7), file("w2", blank, 0.7)], [{ words: parseHash(blank)! }]);
    expect(fresh.map((f) => f.id)).toEqual(["w1", "w2"]);
    expect(skipped).toEqual([]);
  });

  test("inside one batch only the first of a group of look-alikes goes in", () => {
    const base = hashFor(5);
    const batch = [file("a", base, 0.7), file("b", flipBits(base, 6), 0.7), file("c", flipBits(base, 12), 0.7), file("d", hashFor(6), 0.7), file("e", flipBits(hashFor(6), 3), 0.7)];
    const { fresh, skipped } = splitDuplicates(batch, []);
    expect(fresh.map((f) => f.id)).toEqual(["a", "d"]);
    expect(skipped.map((f) => f.id)).toEqual(["b", "c", "e"]);
  });

  test("two thousand known fingerprints and two hundred new files finish at once", () => {
    const known = Array.from({ length: 2000 }, (_, i) => print(hashFor(1000 + i), 0.7));
    const batch = Array.from({ length: 200 }, (_, i) => file("n" + i, i % 10 === 0 ? flipBits(hashFor(1000 + i), 5) : hashFor(5000 + i), 0.7));
    const started = Date.now();
    const { fresh, skipped } = splitDuplicates(batch, known);
    expect(skipped.length).toBe(20);
    expect(fresh.length).toBe(180);
    expect(Date.now() - started).toBeLessThan(2000);
  });
});

describe("what the list compares against", () => {
  test("every item in the list counts, whatever its state, and a removed one does not", () => {
    let board = addFiles(emptyBoard, [file("w", hashFor(1), 0.7), file("u", hashFor(2), 0.7), file("j", hashFor(3), 0.7), file("n")]).board;
    board = startUpload(board, "local:u").board;
    board = uploadSucceeded(startUpload(board, "local:j").board, "local:j", "job-1").board;
    expect(fingerprintsOf(board).length).toBe(3);
    board = removeKeys(board, ["local:w"]).board;
    expect(fingerprintsOf(board).length).toBe(2);
    expect(board.items["job-1"]).toMatchObject({ kind: "job", hash: hashFor(3), aspect: 0.7 });
  });
});