import type { Board } from "./board";
import { canOpenJob } from "./job";

/** 高亮项移动并夹在 [0, count-1]; 从无高亮出发时, 向下选第一个, 向上选最后一个 (网格里一步是一整行, 也一样从头尾开始)。 */
export function moveHighlight(count: number, current: number | null, delta: number): number | null {
  if (count <= 0) return null;
  if (current === null) return delta > 0 ? 0 : count - 1;
  return Math.min(Math.max(current + delta, 0), count - 1);
}

export type Arrow = "up" | "down" | "left" | "right";

/** 上下总是移动; 左右只在查看器打开时与上下等效, 列表里不抢方向键。 */
export function arrowDelta(arrow: Arrow, viewerOpen: boolean): number | null {
  switch (arrow) {
    case "up":
      return -1;
    case "down":
      return 1;
    case "left":
      return viewerOpen ? -1 : null;
    case "right":
      return viewerOpen ? 1 : null;
  }
}

/** 列表页的方向键移动多少项: 单列时上下各一项、左右不抢键; 网格里上下是一整行、左右是一项。 */
export function gridStep(arrow: Arrow, columns: number): number | null {
  switch (arrow) {
    case "up":
      return -columns;
    case "down":
      return columns;
    case "left":
      return columns > 1 ? -1 : null;
    case "right":
      return columns > 1 ? 1 : null;
  }
}

/** 查看器里在已完成的 Job 之间移动; 到头时停在原处。 */
export function adjacentDone(board: Board, currentKey: string | null, delta: number): string | null {
  const done = board.order.filter((key) => {
    const item = board.items[key];
    return item?.kind === "job" && canOpenJob(item.row.state);
  });
  if (done.length === 0) return null;
  const at = currentKey === null ? -1 : done.indexOf(currentKey);
  if (at >= 0) {
    const next = at + delta;
    return done[next] ?? currentKey;
  }
  return delta >= 0 ? done[0]! : done[done.length - 1]!;
}

/** 已完成 Job 中某一项的位置 (从 1 起) 与总数, 查看器角标用。 */
export function donePosition(board: Board, key: string): { index: number; total: number } | null {
  const done = board.order.filter((k) => {
    const item = board.items[k];
    return item?.kind === "job" && canOpenJob(item.row.state);
  });
  const at = done.indexOf(key);
  return at < 0 ? null : { index: at + 1, total: done.length };
}
