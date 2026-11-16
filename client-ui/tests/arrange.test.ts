import { describe, expect, test } from "nsh/test";
import { moveItem, moveToBack, moveToFront, sortBoard, SORT_CHOICES, sortChoiceId, sortChoiceLabel, type SortChoice } from "../src/domain/arrange";
import {
  addFiles,
  applyJobList,
  emptyBoard,
  planUploads,
  removeKeys,
  restoreJobs,
  savedItems,
  startUpload,
  uploadSucceeded,
  type Board,
} from "../src/domain/board";
import { adjacentDone, donePosition } from "../src/domain/nav";
import { decodeSaved, encodeSaved } from "../src/domain/saved";
import type { LocalFile } from "../src/domain/types";

const T = 1_700_000_000_000;
const file = (id: string, name: string, size: number, modifiedAt?: number): LocalFile => ({
  id,
  name,
  size,
  type: "image/png",
  ...(modifiedAt === undefined ? {} : { modifiedAt }),
});
const keys = (board: Board) => board.displayOrder.map((key) => key.replace("local:", ""));
const choice = (field: SortChoice["field"], direction: SortChoice["direction"]): SortChoice => ({ field, direction });
const limits = { capacity: 10, concurrency: 10 };

/** 名字、日期、大小三种顺序互不相同的一组文件。 */
function fixture(): Board {
  return addFiles(emptyBoard, [
    file("a", "page10.png", 300, T + 3000),
    file("b", "page2.png", 100, T + 1000),
    file("c", "Page1.png", 500, T + 5000),
    file("d", "page3.png", 200, T + 2000),
    file("e", "page11.png", 400, T + 4000),
  ]).board;
}

describe("one-time sort", () => {
  test("by name is natural, case-insensitive, in both directions", () => {
    const board = fixture();
    expect(keys(sortBoard(board, choice("name", "asc")).board)).toEqual(["c", "b", "d", "a", "e"]);
    expect(keys(sortBoard(board, choice("name", "desc")).board)).toEqual(["e", "a", "d", "b", "c"]);
  });

  test("by date uses the modification time, oldest first or newest first", () => {
    const board = fixture();
    expect(keys(sortBoard(board, choice("modified", "asc")).board)).toEqual(["b", "d", "a", "e", "c"]);
    expect(keys(sortBoard(board, choice("modified", "desc")).board)).toEqual(["c", "e", "a", "d", "b"]);
  });

  test("by size uses the byte count, smallest first or largest first", () => {
    const board = fixture();
    expect(keys(sortBoard(board, choice("size", "asc")).board)).toEqual(["b", "d", "a", "e", "c"]);
    expect(keys(sortBoard(board, choice("size", "desc")).board)).toEqual(["c", "e", "a", "d", "b"]);
  });

  test("every state is sorted together: pending, uploading and jobs", () => {
    let board = fixture();
    board = uploadSucceeded(startUpload(board, "local:a").board, "local:a", "job-a").board;
    board = applyJobList(board, [{ jobId: "job-a", state: "done" }]).board;
    board = startUpload(board, "local:d").board;
    const sorted = sortBoard(board, choice("size", "asc")).board;
    expect(sorted.displayOrder).toEqual(["local:b", "local:d", "job-a", "local:e", "local:c"]);
    expect(sorted.items["job-a"]?.kind).toBe("job");
    expect(sorted.items["local:d"]?.kind).toBe("uploading");
  });

  test("jobs keep the name, size and date their file had", () => {
    let board = addFiles(emptyBoard, [file("a", "b.png", 20, T), file("b", "a.png", 10, T + 1)]).board;
    board = uploadSucceeded(startUpload(board, "local:a").board, "local:a", "job-a").board;
    board = uploadSucceeded(startUpload(board, "local:b").board, "local:b", "job-b").board;
    expect(sortBoard(board, choice("name", "asc")).board.displayOrder).toEqual(["job-b", "job-a"]);
    expect(sortBoard(board, choice("size", "desc")).board.displayOrder).toEqual(["job-a", "job-b"]);
    expect(sortBoard(board, choice("modified", "desc")).board.displayOrder).toEqual(["job-b", "job-a"]);
  });

  test("items with no value go last whichever way it sorts", () => {
    let board = addFiles(emptyBoard, [file("a", "b.png", 0), file("b", "a.png", 50, T), file("c", "c.png", 10)]).board;
    board = restoreJobsBefore(board, ["old1", "old2"]);
    // size 0 (宿主没读到) 和没有日期的都算缺失
    expect(keys(sortBoard(board, choice("size", "asc")).board)).toEqual(["c", "b", "a", "old1", "old2"]);
    expect(keys(sortBoard(board, choice("size", "desc")).board)).toEqual(["b", "c", "a", "old1", "old2"]);
    expect(keys(sortBoard(board, choice("modified", "asc")).board)).toEqual(["b", "a", "c", "old1", "old2"]);
    expect(keys(sortBoard(board, choice("modified", "desc")).board)).toEqual(["b", "a", "c", "old1", "old2"]);
  });

  test("a restored job saved before names existed sorts after the named ones", () => {
    const restored = restoreJobs([{ id: "x" }, { id: "y", name: "b.png" }, { id: "z", name: "a.png" }], [
      { jobId: "x", state: "done" },
      { jobId: "y", state: "done" },
      { jobId: "z", state: "done" },
    ]);
    expect(sortBoard(restored, choice("name", "asc")).board.displayOrder).toEqual(["z", "y", "x"]);
    expect(sortBoard(restored, choice("name", "desc")).board.displayOrder).toEqual(["y", "z", "x"]);
  });

  test("equal values keep their current order, so descending is not the reverse of ascending", () => {
    const board = addFiles(emptyBoard, [file("a", "x.png", 10), file("b", "y.png", 10), file("c", "z.png", 10), file("d", "w.png", 5)]).board;
    expect(keys(sortBoard(board, choice("size", "asc")).board)).toEqual(["d", "a", "b", "c"]);
    expect(keys(sortBoard(board, choice("size", "desc")).board)).toEqual(["a", "b", "c", "d"]);
    const same = addFiles(emptyBoard, [file("a", "Page1.png", 1), file("b", "page1.png", 1)]).board;
    expect(keys(sortBoard(same, choice("name", "desc")).board)).toEqual(["a", "b"]);
  });

  test("sorting never touches the order files are submitted to the Daemon", () => {
    const board = fixture();
    const before = planUploads(board, { capacity: 3, concurrency: 3 });
    expect(before).toEqual(["local:a", "local:b", "local:c"]);
    for (const c of SORT_CHOICES) {
      const sorted = sortBoard(board, c).board;
      expect(sorted.submitOrder).toBe(board.submitOrder);
      expect(planUploads(sorted, { capacity: 3, concurrency: 3 })).toEqual(before);
    }
    // 排序之后新加入的文件仍然排在提交队列的末尾
    const sorted = sortBoard(board, choice("name", "asc")).board;
    const grown = addFiles(sorted, [file("z", "aaa.png", 1)]).board;
    expect(grown.submitOrder.at(-1)).toBe("local:z");
    expect(planUploads(grown, { capacity: 6, concurrency: 6 }).at(-1)).toBe("local:z");
  });

  test("it is one-time: files added afterwards are appended and nothing re-sorts", () => {
    const sorted = sortBoard(fixture(), choice("name", "asc")).board;
    const grown = addFiles(sorted, [file("y", "a-first.png", 1), file("z", "Page0.png", 1)]).board;
    expect(keys(grown)).toEqual(["c", "b", "d", "a", "e", "y", "z"]);
  });

  test("an order that is already sorted changes nothing", () => {
    const sorted = sortBoard(fixture(), choice("name", "asc"));
    expect(sorted.orderChanged).toBe(true);
    const again = sortBoard(sorted.board, choice("name", "asc"));
    expect(again.orderChanged).toBe(false);
    expect(again.board).toBe(sorted.board);
    expect(sortBoard(emptyBoard, choice("size", "asc")).orderChanged).toBe(false);
  });

  test("the sort reports an order change but no changed rows, so no row signal is rewritten", () => {
    const change = sortBoard(fixture(), choice("size", "desc"));
    expect(change.orderChanged).toBe(true);
    expect(change.changed).toEqual([]);
  });

  test("the six menu entries are distinct and labelled", () => {
    expect(SORT_CHOICES.length).toBe(6);
    expect(new Set(SORT_CHOICES.map(sortChoiceId)).size).toBe(6);
    expect(SORT_CHOICES.map(sortChoiceLabel)).toEqual([
      "名称 · 升序",
      "名称 · 降序",
      "日期 · 从旧到新",
      "日期 · 从新到旧",
      "大小 · 从小到大",
      "大小 · 从大到小",
    ]);
  });
});

/** 给 board 末尾接上两个恢复出来、没有任何文件信息的 Job (保存时还没有这些字段)。 */
function restoreJobsBefore(board: Board, ids: string[]): Board {
  const restored = restoreJobs(ids.map((id) => ({ id })), ids.map((jobId) => ({ jobId, state: "done" as const })));
  return {
    submitOrder: [...board.submitOrder, ...restored.submitOrder],
    displayOrder: [...board.displayOrder, ...restored.displayOrder],
    items: { ...board.items, ...restored.items },
  };
}

describe("moving one item", () => {
  test("to the front, to the back and to a position, the rest make room", () => {
    const board = fixture();
    expect(keys(moveToFront(board, "local:d").board)).toEqual(["d", "a", "b", "c", "e"]);
    expect(keys(moveToBack(board, "local:b").board)).toEqual(["a", "c", "d", "e", "b"]);
    expect(keys(moveItem(board, "local:a", 3).board)).toEqual(["b", "c", "d", "a", "e"]);
    expect(keys(moveItem(board, "local:e", 1).board)).toEqual(["a", "e", "b", "c", "d"]);
  });

  test("positions beyond either end clamp, and an unmoved or unknown item changes nothing", () => {
    const board = fixture();
    expect(keys(moveItem(board, "local:c", 99).board)).toEqual(["a", "b", "d", "e", "c"]);
    expect(keys(moveItem(board, "local:c", -4).board)).toEqual(["c", "a", "b", "d", "e"]);
    for (const change of [moveToFront(board, "local:a"), moveToBack(board, "local:e"), moveItem(board, "local:c", 2), moveItem(board, "nope", 1)]) {
      expect(change.orderChanged).toBe(false);
      expect(change.board).toBe(board);
    }
  });

  test("moving leaves the submit order, the items and the other rows alone", () => {
    const board = fixture();
    const moved = moveToFront(board, "local:e");
    expect(moved.board.submitOrder).toBe(board.submitOrder);
    expect(moved.board.items).toBe(board.items);
    expect(moved.changed).toEqual([]);
    expect(moved.orderChanged).toBe(true);
    expect(planUploads(moved.board, limits)).toEqual(planUploads(board, limits));
  });

  test("a moved item that uploads keeps the place it was moved to", () => {
    let board = moveToFront(fixture(), "local:e").board;
    board = uploadSucceeded(startUpload(board, "local:e").board, "local:e", "job-e").board;
    expect(board.displayOrder).toEqual(["job-e", "local:a", "local:b", "local:c", "local:d"]);
    expect(board.submitOrder).toEqual(["local:a", "local:b", "local:c", "local:d", "job-e"]);
  });

  test("removing an item removes it from both orders", () => {
    const board = removeKeys(moveToFront(fixture(), "local:c").board, ["local:c", "local:a"]).board;
    expect(board.displayOrder).toEqual(["local:b", "local:d", "local:e"]);
    expect(board.submitOrder).toEqual(["local:b", "local:d", "local:e"]);
  });
});

describe("the displayed order drives navigation and persistence", () => {
  function doneBoard(): Board {
    let board = addFiles(emptyBoard, [file("a", "c.png", 1, T), file("b", "a.png", 2, T + 1), file("c", "b.png", 3, T + 2)]).board;
    for (const id of ["a", "b", "c"]) board = uploadSucceeded(startUpload(board, "local:" + id).board, "local:" + id, "job-" + id).board;
    return applyJobList(board, ["a", "b", "c"].map((id) => ({ jobId: "job-" + id, state: "done" as const }))).board;
  }

  test("the viewer steps through finished jobs in the order shown", () => {
    const board = sortBoard(doneBoard(), choice("name", "asc")).board;
    expect(board.displayOrder).toEqual(["job-b", "job-c", "job-a"]);
    expect(adjacentDone(board, "job-b", 1)).toBe("job-c");
    expect(adjacentDone(board, "job-c", 1)).toBe("job-a");
    expect(adjacentDone(board, "job-a", 1)).toBe("job-a");
    expect(adjacentDone(board, null, 1)).toBe("job-b");
    expect(donePosition(board, "job-a")).toEqual({ index: 3, total: 3 });
  });

  test("the saved list is in display order with name, size and date, and restores into the same order", () => {
    const board = moveToBack(sortBoard(doneBoard(), choice("size", "desc")).board, "job-c").board;
    expect(board.displayOrder).toEqual(["job-b", "job-a", "job-c"]);
    const saved = savedItems(board);
    expect(saved).toEqual([
      { id: "job-b", name: "a.png", size: 2, modifiedAt: T + 1 },
      { id: "job-a", name: "c.png", size: 1, modifiedAt: T },
      { id: "job-c", name: "b.png", size: 3, modifiedAt: T + 2 },
    ]);
    const restored = restoreJobs(decodeSaved(encodeSaved(saved)), ["a", "b", "c"].map((id) => ({ jobId: "job-" + id, state: "done" as const })));
    expect(restored.displayOrder).toEqual(board.displayOrder);
    expect(sortBoard(restored, choice("name", "asc")).board.displayOrder).toEqual(["job-b", "job-c", "job-a"]);
    expect(sortBoard(restored, choice("size", "asc")).board.displayOrder).toEqual(["job-a", "job-b", "job-c"]);
  });
});