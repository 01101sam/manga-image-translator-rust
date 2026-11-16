import { describe, expect, test } from "nsh/test";
import { moveItem, moveToBack, moveToFront, sortBoard, SORT_CHOICES } from "../src/domain/arrange";
import {
  addFiles,
  applyJobList,
  boardIssues,
  clearableKeys,
  emptyBoard,
  removeKeys,
  repairBoard,
  retryRejected,
  startUpload,
  uploadRejected,
  uploadRequeued,
  uploadSucceeded,
  type Board,
  type Change,
} from "../src/domain/board";
import type { JobEntry, LocalFile } from "../src/domain/types";

/** 固定种子的伪随机数, 失败时能原样重放。 */
function lcg(seed: number) {
  let state = seed >>> 0;
  return (n: number) => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state % n;
  };
}

const file = (id: string): LocalFile => ({ id, name: id + ".png", size: 10 + id.length, type: "image/png" });

describe("Board invariants", () => {
  test("no sequence of operations leaves a key without an item or an item out of an order", () => {
    for (let seed = 1; seed <= 40; seed++) {
      const pick = lcg(seed);
      let board: Board = emptyBoard;
      let nextFile = 0;
      let nextJob = 0;
      const log: string[] = [];
      const apply = (name: string, change: Change) => {
        board = change.board;
        log.push(name);
        expect({ seed, log: log.slice(-6), issues: boardIssues(board) }).toEqual({ seed, log: log.slice(-6), issues: [] });
      };
      const keysOf = (kind: string) => board.displayOrder.filter((key) => board.items[key]?.kind === kind);
      for (let step = 0; step < 120; step++) {
        const kind = pick(11);
        const any = board.displayOrder;
        const choose = (keys: readonly string[]) => keys[pick(keys.length)];
        if (kind === 0 || any.length === 0) {
          apply("add", addFiles(board, Array.from({ length: 1 + pick(5) }, () => file("f" + nextFile++))));
        } else if (kind === 1) {
          const key = choose(keysOf("pending"));
          if (key) apply("start", startUpload(board, key));
        } else if (kind === 2) {
          const key = choose(keysOf("uploading"));
          if (key) apply("succeeded", uploadSucceeded(board, key, "job-" + nextJob++));
        } else if (kind === 3) {
          const key = choose(keysOf("uploading"));
          if (key) apply("requeued", uploadRequeued(board, key));
        } else if (kind === 4) {
          const key = choose(keysOf("uploading"));
          if (key) apply("rejected", uploadRejected(board, key, "no"));
        } else if (kind === 5) {
          const key = choose(any);
          if (key) apply("retry", retryRejected(board, key));
        } else if (kind === 6) {
          const doomed = any.filter(() => pick(4) === 0);
          apply("remove", removeKeys(board, doomed));
        } else if (kind === 7) {
          apply("sort", sortBoard(board, SORT_CHOICES[pick(SORT_CHOICES.length)]!));
        } else if (kind === 8) {
          const key = choose(any);
          if (key) apply("move", moveItem(board, key, pick(any.length)));
        } else if (kind === 9) {
          const key = choose(any);
          if (key) apply(pick(2) === 0 ? "front" : "back", pick(2) === 0 ? moveToFront(board, key) : moveToBack(board, key));
        } else {
          const entries: JobEntry[] = keysOf("job").map((key) => ({ jobId: key, state: pick(3) === 0 ? "done" : "running" }));
          apply("list", applyJobList(board, entries));
          apply("clear", removeKeys(board, clearableKeys(board)));
        }
      }
    }
  });

  test("a dangling key, a duplicate and an orphan item are all reported, and the repair removes every one", () => {
    const base = addFiles(emptyBoard, [file("a"), file("b")]).board;
    const broken: Board = {
      submitOrder: ["local:a", "local:a", "local:ghost"],
      displayOrder: ["local:ghost", "local:b"],
      items: base.items,
    };
    expect(boardIssues(broken).length).toBeGreaterThanOrEqual(4);
    const fixed = repairBoard(broken);
    expect(boardIssues(fixed)).toEqual([]);
    expect([...fixed.submitOrder].sort()).toEqual(["local:a", "local:b"]);
    expect([...fixed.displayOrder].sort()).toEqual(["local:a", "local:b"]);
  });
});
