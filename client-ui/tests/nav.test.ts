import { describe, expect, test } from "nsh/test";
import { addFiles, applyJobList, emptyBoard, uploadSucceeded, startUpload, type Board } from "../src/domain/board";
import { adjacentDone, arrowDelta, donePosition, moveHighlight } from "../src/domain/nav";
import type { JobEntry, JobState, LocalFile } from "../src/domain/types";

const file = (id: string): LocalFile => ({ id, name: id + ".png", size: 10, type: "image/png" });

/** 造一个含若干 Job 的 Board: 逐个加文件、上传、拿到 jobId。 */
function boardOf(...states: [string, JobState][]): Board {
  let board = emptyBoard;
  for (const [id] of states) {
    board = addFiles(board, [file(id)]).board;
    board = startUpload(board, "local:" + id).board;
    board = uploadSucceeded(board, "local:" + id, id).board;
  }
  const entries: JobEntry[] = states.map(([jobId, state]) => ({ jobId, state }));
  return applyJobList(board, entries).board;
}

describe("navigation", () => {
  test("a grid row step from no highlight also starts at the edge, then moves a row and clamps", () => {
    expect(moveHighlight(10, null, 4)).toBe(0);
    expect(moveHighlight(10, null, -4)).toBe(9);
    expect(moveHighlight(10, 3, 4)).toBe(7);
    expect(moveHighlight(10, 8, 4)).toBe(9);
    expect(moveHighlight(10, 1, -4)).toBe(0);
  });

  test("highlight starts at the edge and clamps", () => {
    expect(moveHighlight(3, null, 1)).toBe(0);
    expect(moveHighlight(3, null, -1)).toBe(2);
    expect(moveHighlight(0, null, 1)).toBeNull();
    expect(moveHighlight(3, 0, -1)).toBe(0);
    expect(moveHighlight(3, 2, 1)).toBe(2);
    expect(moveHighlight(3, 1, 1)).toBe(2);
  });

  test("vertical arrows always move; horizontal only while the viewer is open", () => {
    expect(arrowDelta("up", false)).toBe(-1);
    expect(arrowDelta("down", false)).toBe(1);
    expect(arrowDelta("left", false)).toBeNull();
    expect(arrowDelta("right", false)).toBeNull();
    expect(arrowDelta("left", true)).toBe(-1);
    expect(arrowDelta("right", true)).toBe(1);
  });

  test("the viewer skips jobs that are not done and stops at the ends", () => {
    const board = boardOf(["a", "queued"], ["b", "done"], ["c", "running"], ["d", "done"]);
    expect(adjacentDone(board, "b", 1)).toBe("d");
    expect(adjacentDone(board, "d", -1)).toBe("b");
    expect(adjacentDone(board, "d", 1)).toBe("d");
    expect(adjacentDone(board, "c", 1)).toBe("b");
    expect(adjacentDone(board, null, 1)).toBe("b");
    expect(adjacentDone(board, null, -1)).toBe("d");
    expect(donePosition(board, "d")).toEqual({ index: 2, total: 2 });
  });

  test("no done jobs means nowhere to go", () => {
    expect(adjacentDone(boardOf(["a", "queued"]), null, 1)).toBeNull();
  });
});
