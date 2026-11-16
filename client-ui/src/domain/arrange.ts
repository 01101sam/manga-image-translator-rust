import { factsOf, unchanged, type Board, type Change } from "./board";
import { compareNaturalKeys, naturalKey } from "./natural";

/**
 * 任务页的一次性排序与手动调整。只改 `displayOrder`: 提交给 Daemon 的先后 (`submitOrder`) 和 Daemon 的队列都不受影响,
 * 之后新加入的条目照常追加在末尾, 不会自动维持排序。
 */

export type SortField = "name" | "modified" | "size";
export type SortDirection = "asc" | "desc";

export interface SortChoice {
  readonly field: SortField;
  readonly direction: SortDirection;
}

export const SORT_FIELD_LABELS: Record<SortField, string> = { name: "名称", modified: "日期", size: "大小" };

/** 菜单里的六项, 每个字段先升序后降序。 */
export const SORT_CHOICES: readonly SortChoice[] = [
  { field: "name", direction: "asc" },
  { field: "name", direction: "desc" },
  { field: "modified", direction: "asc" },
  { field: "modified", direction: "desc" },
  { field: "size", direction: "asc" },
  { field: "size", direction: "desc" },
];

const DIRECTION_LABELS: Record<SortField, Record<SortDirection, string>> = {
  name: { asc: "升序", desc: "降序" },
  modified: { asc: "从旧到新", desc: "从新到旧" },
  size: { asc: "从小到大", desc: "从大到小" },
};

export const sortChoiceLabel = (choice: SortChoice): string => `${SORT_FIELD_LABELS[choice.field]} · ${DIRECTION_LABELS[choice.field][choice.direction]}`;

export const sortChoiceId = (choice: SortChoice): string => `${choice.field}-${choice.direction}`;

type SortValue = ReturnType<typeof naturalKey> | number;

function valueOf(board: Board, key: string, field: SortField): SortValue | undefined {
  const item = board.items[key];
  if (item === undefined) return undefined;
  const facts = factsOf(item);
  if (field === "name") return facts.name === undefined ? undefined : naturalKey(facts.name);
  if (field === "size") return facts.size;
  return facts.modifiedAt;
}

function compareValues(a: SortValue, b: SortValue): number {
  if (typeof a === "number" && typeof b === "number") return a < b ? -1 : a > b ? 1 : 0;
  return compareNaturalKeys(a as ReturnType<typeof naturalKey>, b as ReturnType<typeof naturalKey>);
}

/**
 * 把当前所有条目 (不分状态) 排一次。缺失值的条目不论升降都在末尾; 值相同的保持现有的相对顺序,
 * 所以降序不是升序的倒置。
 */
export function sortBoard(board: Board, choice: SortChoice): Change {
  const sign = choice.direction === "asc" ? 1 : -1;
  const rows = board.displayOrder.map((key, index) => ({ key, index, value: valueOf(board, key, choice.field) }));
  rows.sort((a, b) => {
    if (a.value === undefined || b.value === undefined) {
      if (a.value === b.value) return a.index - b.index;
      return a.value === undefined ? 1 : -1;
    }
    return compareValues(a.value, b.value) * sign || a.index - b.index;
  });
  const displayOrder = rows.map((row) => row.key);
  if (displayOrder.every((key, index) => key === board.displayOrder[index])) return unchanged(board);
  return { board: { ...board, displayOrder }, changed: [], orderChanged: true };
}

/** 把一项移到显示顺序的第 `toIndex` 位 (夹在首尾之间), 其余项顺次让位。 */
export function moveItem(board: Board, key: string, toIndex: number): Change {
  const from = board.displayOrder.indexOf(key);
  if (from < 0) return unchanged(board);
  const to = Math.min(Math.max(Math.trunc(toIndex), 0), board.displayOrder.length - 1);
  if (to === from) return unchanged(board);
  const displayOrder = [...board.displayOrder];
  displayOrder.splice(from, 1);
  displayOrder.splice(to, 0, key);
  return { board: { ...board, displayOrder }, changed: [], orderChanged: true };
}

export const moveToFront = (board: Board, key: string): Change => moveItem(board, key, 0);

export const moveToBack = (board: Board, key: string): Change => moveItem(board, key, board.displayOrder.length - 1);
