import { describe, expect, test } from "nsh/test";
import {
  addFiles,
  applyJobEvent,
  boardIssues,
  applyJobList,
  clearableKeys,
  isRemovable,
  countItems,
  emptyBoard,
  inFlight,
  jobsNeedingError,
  planUploads,
  removeKeys,
  restoreBoard,
  restoreJobs,
  savedItems,
  retryRejected,
  startUpload,
  uploadRejected,
  uploadRequeued,
  uploadSucceeded,
  type Board,
} from "../src/domain/board";
import type { JobEntry, LocalFile } from "../src/domain/types";

const file = (id: string): LocalFile => ({ id, name: id + ".png", size: 10, type: "image/png" });
const files = (n: number) => Array.from({ length: n }, (_, i) => file("f" + i));
const limits = { capacity: 4, concurrency: 2 };

function submitted(board: Board, id: string, jobId: string): Board {
  const started = startUpload(board, "local:" + id).board;
  return uploadSucceeded(started, "local:" + id, jobId).board;
}

describe("Board", () => {
  test("added files queue locally in order and never duplicate", () => {
    let change = addFiles(emptyBoard, files(3));
    expect(change.board.submitOrder).toEqual(["local:f0", "local:f1", "local:f2"]);
    expect(change.board.displayOrder).toEqual(change.board.submitOrder);
    expect(change.changed.length).toBe(3);
    change = addFiles(change.board, [file("f1"), file("f9")]);
    expect(change.board.submitOrder.at(-1)).toBe("local:f9");
    expect(change.board.submitOrder.length).toBe(4);
    expect(change.board.displayOrder.length).toBe(4);
  });

  test("uploads start in order and respect upload concurrency", () => {
    const board = addFiles(emptyBoard, files(5)).board;
    expect(planUploads(board, limits)).toEqual(["local:f0", "local:f1"]);
    const started = startUpload(startUpload(board, "local:f0").board, "local:f1").board;
    expect(planUploads(started, limits)).toEqual([]);
  });

  test("in-flight jobs plus uploads never exceed the queue capacity", () => {
    let board = addFiles(emptyBoard, files(8)).board;
    for (let i = 0; i < 3; i++) board = submitted(board, "f" + i, "job" + i);
    board = applyJobList(board, [0, 1, 2].map((i) => ({ jobId: "job" + i, state: "queued" }))).board;
    expect(inFlight(board)).toBe(3);
    expect(planUploads(board, limits)).toEqual(["local:f3"]);
    board = startUpload(board, "local:f3").board;
    expect(inFlight(board)).toBe(4);
    expect(planUploads(board, { capacity: 4, concurrency: 9 })).toEqual([]);
  });

  test("finishing jobs frees queue room for the next pending file", () => {
    let board = addFiles(emptyBoard, files(3)).board;
    board = submitted(submitted(board, "f0", "j0"), "f1", "j1");
    expect(planUploads(board, { capacity: 2, concurrency: 3 })).toEqual([]);
    board = applyJobList(board, [{ jobId: "j0", state: "done" }, { jobId: "j1", state: "running" }]).board;
    expect(planUploads(board, { capacity: 2, concurrency: 3 })).toEqual(["local:f2"]);
  });

  test("a finished upload swaps the local key for the job id in place", () => {
    const board = addFiles(emptyBoard, files(3)).board;
    const next = submitted(board, "f1", "job-1");
    expect(next.submitOrder).toEqual(["local:f0", "job-1", "local:f2"]);
    expect(next.displayOrder).toEqual(["local:f0", "job-1", "local:f2"]);
    const item = next.items["job-1"];
    expect(item?.kind === "job" && item.name).toBe("f1.png");
    expect(next.items["local:f1"]).toBeUndefined();
  });

  test("queue full, engine stopped and network errors put the file back as pending", () => {
    const board = startUpload(addFiles(emptyBoard, files(1)).board, "local:f0").board;
    const back = uploadRequeued(board, "local:f0").board;
    expect(back.items["local:f0"]?.kind).toBe("pending");
    expect(planUploads(back, limits)).toEqual(["local:f0"]);
  });

  test("a rejected file is not retried automatically but can be retried by hand", () => {
    const board = startUpload(addFiles(emptyBoard, files(1)).board, "local:f0").board;
    const rejected = uploadRejected(board, "local:f0", "不是图片").board;
    expect(planUploads(rejected, limits)).toEqual([]);
    expect(clearableKeys(rejected)).toEqual(["local:f0"]);
    expect(planUploads(retryRejected(rejected, "local:f0").board, limits)).toEqual(["local:f0"]);
  });

  test("the batch update returns only the rows that changed, with untouched rows keeping their identity", () => {
    let board = addFiles(emptyBoard, files(3)).board;
    for (let i = 0; i < 3; i++) board = submitted(board, "f" + i, "j" + i);
    const entries: JobEntry[] = [
      { jobId: "j0", state: "queued" },
      { jobId: "j1", state: "queued" },
      { jobId: "j2", state: "queued" },
    ];
    const first = applyJobList(board, entries);
    expect(first.changed).toEqual([]);
    expect(first.board).toBe(board);
    const second = applyJobList(board, [entries[0]!, { jobId: "j1", state: "running" }, entries[2]!]);
    expect(second.changed).toEqual(["j1"]);
    expect(second.board.items["j0"]).toBe(board.items["j0"]);
    expect(second.board.items["j2"]).toBe(board.items["j2"]);
    expect(second.board.submitOrder).toBe(board.submitOrder);
    expect(second.board.displayOrder).toBe(board.displayOrder);
  });

  test("the daemon's arbitrary order never reorders the list", () => {
    let board = addFiles(emptyBoard, files(3)).board;
    for (let i = 0; i < 3; i++) board = submitted(board, "f" + i, "j" + i);
    const shuffled: JobEntry[] = [
      { jobId: "j2", state: "done" },
      { jobId: "j0", state: "done" },
      { jobId: "j1", state: "done" },
    ];
    expect(applyJobList(board, shuffled).board.displayOrder).toEqual(["j0", "j1", "j2"]);
  });

  test("jobs other clients submitted are not adopted", () => {
    const board = submitted(addFiles(emptyBoard, files(1)).board, "f0", "mine");
    const next = applyJobList(board, [{ jobId: "mine", state: "queued" }, { jobId: "theirs", state: "queued" }]);
    expect(next.board.displayOrder).toEqual(["mine"]);
  });

  test("a vanished job fails once and its reason is fetched separately", () => {
    let board = submitted(addFiles(emptyBoard, files(1)).board, "f0", "j0");
    board = applyJobList(board, []).board;
    expect(board.items["j0"]?.kind === "job" && board.items["j0"].row.state).toBe("failed");
    expect(jobsNeedingError(applyJobList(submitted(addFiles(emptyBoard, files(1)).board, "f0", "j1"), [{ jobId: "j1", state: "failed" }]).board)).toEqual(["j1"]);
  });

  test("cancelling a queued job goes through the job machine", () => {
    let board = submitted(addFiles(emptyBoard, files(1)).board, "f0", "j0");
    const { change, command } = applyJobEvent(board, "j0", { type: "requestCancel" });
    board = change.board;
    expect(command).toEqual({ type: "cancel", jobId: "j0" });
    board = applyJobEvent(board, "j0", { type: "cancelAccepted" }).change.board;
    expect(board.items["j0"]?.kind === "job" && board.items["j0"].row.state).toBe("cancelled");
  });

  test("clearing finished removes ended jobs only", () => {
    let board = addFiles(emptyBoard, files(4)).board;
    for (let i = 0; i < 3; i++) board = submitted(board, "f" + i, "j" + i);
    board = applyJobList(board, [{ jobId: "j0", state: "done" }, { jobId: "j1", state: "running" }, { jobId: "j2", state: "failed" }]).board;
    expect(clearableKeys(board)).toEqual(["j0", "j2"]);
    const cleared = removeKeys(board, clearableKeys(board)).board;
    expect(cleared.submitOrder).toEqual(["j1", "local:f3"]);
    expect(cleared.displayOrder).toEqual(["j1", "local:f3"]);
  });

  test("only finished jobs and files not yet in flight are removable", () => {
    let board = addFiles(emptyBoard, files(3)).board;
    board = submitted(submitted(board, "f0", "j0"), "f1", "j1");
    board = applyJobList(board, [{ jobId: "j0", state: "queued" }, { jobId: "j1", state: "done" }]).board;
    expect(isRemovable(board.items["j0"])).toBe(false);
    expect(isRemovable(board.items["j1"])).toBe(true);
    expect(isRemovable(board.items["local:f2"])).toBe(true);
    expect(isRemovable(startUpload(board, "local:f2").board.items["local:f2"])).toBe(false);
    expect(isRemovable(undefined)).toBe(false);
  });

  test("restoring keeps the saved order and drops jobs the daemon forgot", () => {
    const restored = restoreJobs([{ id: "b", name: "b.png" }, { id: "gone" }, { id: "a" }], [
      { jobId: "a", state: "done" },
      { jobId: "b", state: "queued" },
      { jobId: "stranger", state: "queued" },
    ]);
    expect(restored.displayOrder).toEqual(["b", "a"]);
    expect(restored.submitOrder).toEqual(["b", "a"]);
    expect(restored.items["b"]?.kind === "job" && restored.items["b"].name).toBe("b.png");
  });

  test("counts treat a rejected pending file as failed", () => {
    let board = addFiles(emptyBoard, files(2)).board;
    board = uploadRejected(startUpload(board, "local:f0").board, "local:f0", "x").board;
    const counts = countItems(board);
    expect(counts.failed).toBe(1);
    expect(counts.pending).toBe(1);
    expect(counts.total).toBe(2);
  });

  test("two hundred files plan batches that never overshoot the capacity", () => {
    let board = addFiles(emptyBoard, files(200)).board;
    let submittedCount = 0;
    for (let round = 0; round < 400 && submittedCount < 200; round++) {
      for (const key of planUploads(board, { capacity: 64, concurrency: 3 })) {
        board = uploadSucceeded(startUpload(board, key).board, key, "job-" + key).board;
        submittedCount++;
      }
      expect(inFlight(board)).toBeLessThanOrEqual(64);
      // 模拟 Daemon 完成最老的 10 个
      const active = board.submitOrder.filter((k) => board.items[k]?.kind === "job" && ["queued", "running"].includes((board.items[k] as { row: { state: string } }).row.state));
      board = applyJobList(board, active.slice(0, 10).map((jobId) => ({ jobId, state: "done" as const }))).board;
    }
    expect(submittedCount).toBe(200);
  });

  test("waiting, uploading and refused files are saved with the Jobs in display order, and come back as Pending", () => {
    let board = addFiles(emptyBoard, [file("w"), file("u"), file("r"), file("j")]).board;
    board = startUpload(board, "local:u").board;
    board = startUpload(board, "local:r").board;
    board = uploadRejected(board, "local:r", "格式不支持").board;
    board = submitted(board, "j", "job-9");
    const saved = savedItems(board);
    expect(saved.map((row) => row.id)).toEqual(["local:w", "local:u", "local:r", "job-9"]);
    expect(saved[2]).toEqual({ id: "local:r", name: "r.png", size: 10, modifiedAt: undefined, type: "image/png", rejected: "格式不支持" });
    expect("rejected" in saved[1]!).toBe(false);

    const found = new Map(["w", "u", "r"].map((id) => [id, { ...file(id), thumb: "data:image/jpeg;base64,AAA" }] as const));
    const { board: restored, lost } = restoreBoard(saved, [{ jobId: "job-9", state: "done" }], found);
    expect(lost).toBe(0);
    expect(restored.displayOrder).toEqual(["local:w", "local:u", "local:r", "job-9"]);
    expect(restored.items["local:u"]).toMatchObject({ kind: "pending", file: { id: "u", thumb: "data:image/jpeg;base64,AAA" } });
    expect(restored.items["local:r"]).toMatchObject({ kind: "pending", rejected: "格式不支持" });
    expect("rejected" in restored.items["local:w"]!).toBe(false);
    expect(restored.items["job-9"]!.kind).toBe("job");
    expect(boardIssues(restored)).toEqual([]);
  });

  test("a saved file the host no longer has is dropped and counted, and a Job the Daemon forgot is dropped silently", () => {
    const saved = [{ id: "local:gone" }, { id: "local:here", name: "here.png" }, { id: "job-old" }];
    const { board, lost } = restoreBoard(saved, [], new Map([["here", file("here")]]));
    expect(lost).toBe(1);
    expect(board.displayOrder).toEqual(["local:here"]);
    expect(restoreJobs(saved, []).displayOrder).toEqual([]);
  });
});
