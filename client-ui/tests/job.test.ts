import { describe, expect, test } from "nsh/test";
import { canCancelJob, canOpenJob, jobStatusLabel, reduceJob, shortJobId, type JobRow } from "../src/domain/job";
import type { JobState } from "../src/domain/types";

const row = (jobId: string, state: JobState, extra: Partial<JobRow> = {}): JobRow => ({ jobId, state, cancelPending: false, ...extra });

describe("Job", () => {
  test("queued to running to done", () => {
    let current = row("j1", "queued", { positionInQueue: 2 });
    [current] = reduceJob(current, { type: "snapshot", entry: { jobId: "j1", state: "running" } });
    expect(current.state).toBe("running");
    expect(current.positionInQueue).toBeUndefined();
    [current] = reduceJob(current, { type: "snapshot", entry: { jobId: "j1", state: "done" } });
    expect(current).toEqual(row("j1", "done"));
  });

  test("an unchanged snapshot returns the same row so nothing re-renders", () => {
    const current = row("j1", "running");
    const [next] = reduceJob(current, { type: "snapshot", entry: { jobId: "j1", state: "running" } });
    expect(next).toBe(current);
  });

  test("cancel is only possible while queued", () => {
    const [pending, command] = reduceJob(row("j2", "queued"), { type: "requestCancel" });
    expect(command).toEqual({ type: "cancel", jobId: "j2" });
    expect(pending.cancelPending).toBe(true);
    for (const state of ["running", "done", "failed", "cancelled"] as const) {
      const start = row("j2", state);
      const [next, cmd] = reduceJob(start, { type: "requestCancel" });
      expect(cmd).toEqual({ type: "none" });
      expect(next).toBe(start);
      expect(canCancelJob(state)).toBe(false);
    }
  });

  test("cancel accepted marks cancelled and clears the position", () => {
    const [next] = reduceJob(row("j3", "queued", { positionInQueue: 3, cancelPending: true }), { type: "cancelAccepted" });
    expect(next.state).toBe("cancelled");
    expect(next.positionInQueue).toBeUndefined();
    expect(next.cancelPending).toBe(false);
  });

  test("cancel rejected clears pending and keeps the state", () => {
    const [next] = reduceJob(row("j4", "queued", { cancelPending: true }), { type: "cancelRejected" });
    expect(next.state).toBe("queued");
    expect(next.cancelPending).toBe(false);
  });

  test("a second cancel while pending is ignored", () => {
    const start = row("j5", "queued", { cancelPending: true });
    const [next, command] = reduceJob(start, { type: "requestCancel" });
    expect(command).toEqual({ type: "none" });
    expect(next).toBe(start);
  });

  test("a snapshot that moves past queued drops cancelPending", () => {
    const [next] = reduceJob(row("j6", "queued", { cancelPending: true }), { type: "snapshot", entry: { jobId: "j6", state: "running" } });
    expect(next.cancelPending).toBe(false);
  });

  test("a job the Daemon no longer knows fails if it was active, and is left alone once finished", () => {
    expect(reduceJob(row("j7", "running"), { type: "missing" })[0].state).toBe("failed");
    const done = row("j8", "done");
    expect(reduceJob(done, { type: "missing" })[0]).toBe(done);
  });

  test("the failure reason is fetched once and survives later snapshots", () => {
    let current = row("j9", "failed");
    [current] = reduceJob(current, { type: "errorFetched", error: "模型未加载" });
    [current] = reduceJob(current, { type: "snapshot", entry: { jobId: "j9", state: "failed" } });
    expect(jobStatusLabel(current)).toBe("失败 · 模型未加载");
  });

  test("labels", () => {
    expect(jobStatusLabel(row("a", "queued", { positionInQueue: 3 }))).toBe("排队中 · 第 3 位");
    expect(jobStatusLabel(row("a", "queued"))).toBe("排队中");
    expect(jobStatusLabel(row("a", "running"))).toBe("翻译中");
    expect(jobStatusLabel(row("a", "done"))).toBe("已完成");
    expect(jobStatusLabel(row("a", "failed"))).toBe("失败");
    expect(jobStatusLabel(row("a", "cancelled"))).toBe("已取消");
  });

  test("short id uses the first UUID segment", () => {
    expect(shortJobId("a1b2c3d4-e5f6-7890-abcd-ef1234567890")).toBe("a1b2c3d4");
    expect(shortJobId("solo")).toBe("solo");
  });

  test("only done jobs open", () => {
    expect(canOpenJob("done")).toBe(true);
    for (const state of ["queued", "running", "failed", "cancelled"] as const) expect(canOpenJob(state)).toBe(false);
  });
});
