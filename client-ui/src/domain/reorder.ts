import { GRID_GAP, ROW_HEIGHT, tileHeight, tileWidth } from "./grid";

/**
 * 拖动重排用的几何。列表和网格的单元格大小都是固定的 (grid.ts), 所以由行号就能算出每个单元格在内容里的位置,
 * 不用去量虚拟化列表里正好挂着的那几个 cell (cell 会被复用, 拖动时还会滚出屏幕)。坐标都是内容坐标: 原点在内容左上角, 随滚动平移。
 */

export interface Metrics {
  /** 1 表示一行一项的列表。 */
  readonly columns: number;
  readonly cellWidth: number;
  readonly cellHeight: number;
  /** 单元格之间的间距。 */
  readonly gap: number;
  /** 内容四周的留白。 */
  readonly padding: number;
}

export interface Rect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

export interface Point {
  readonly x: number;
  readonly y: number;
}

export function metricsFor(listWidth: number, columns: number): Metrics {
  if (columns <= 1) return { columns: 1, cellWidth: listWidth, cellHeight: ROW_HEIGHT, gap: 0, padding: 0 };
  const cellWidth = tileWidth(listWidth, columns);
  return { columns, cellWidth, cellHeight: tileHeight(cellWidth), gap: GRID_GAP, padding: GRID_GAP };
}

export function cellRect(m: Metrics, index: number): Rect {
  const column = index % m.columns;
  const row = Math.floor(index / m.columns);
  return {
    x: m.padding + column * (m.cellWidth + m.gap),
    y: m.padding + row * (m.cellHeight + m.gap),
    width: m.cellWidth,
    height: m.cellHeight,
  };
}

export function contentHeight(m: Metrics, count: number): number {
  if (count <= 0) return 0;
  const rows = Math.ceil(count / m.columns);
  return m.padding * 2 + rows * m.cellHeight + (rows - 1) * m.gap;
}

/** 内容坐标 (x, y) 落在第几个单元格上; 在留白、间隙或最后一行之后都落到最近的一格, 夹在 [0, count - 1]。 */
export function slotAt(m: Metrics, count: number, point: Point): number {
  if (count <= 0) return -1;
  const column = Math.min(Math.max(Math.floor((point.x - m.padding) / (m.cellWidth + m.gap)), 0), m.columns - 1);
  const row = Math.max(Math.floor((point.y - m.padding) / (m.cellHeight + m.gap)), 0);
  return Math.min(row * m.columns + column, count - 1);
}

/**
 * 放手后被拖的那项会落在第 to 位: 它占据 to 位置原来那一格, 夹在中间的各项顺次让位。
 * 指示线画在那一格靠 from 的反面: 往后拖 (to > from) 画在它的后缘, 往前拖画在前缘, 所以指示线就是被拖项落下的边。
 * 列表里是一条横线, 网格里是一条竖线; to 等于 from 时没有指示线。
 */
export function dropMarker(m: Metrics, from: number, to: number): Rect | null {
  if (to === from || to < 0) return null;
  const cell = cellRect(m, to);
  const thickness = 4;
  if (m.columns <= 1) {
    const y = to > from ? cell.y + cell.height : cell.y;
    return { x: 0, y: y - thickness / 2, width: cell.width, height: thickness };
  }
  const x = to > from ? cell.x + cell.width + m.gap / 2 : cell.x - m.gap / 2;
  return { x: x - thickness / 2, y: cell.y, width: thickness, height: cell.height };
}

export const EDGE_ZONE = 72;
export const EDGE_MAX_SPEED = 1100;

/** 指针在视口里的纵向位置对应的自动滚动速度 (点 / 秒): 靠近上沿为负、下沿为正, 越靠边越快, 中间为 0。 */
export function edgeScrollSpeed(pointerY: number, viewportHeight: number): number {
  if (viewportHeight <= EDGE_ZONE * 2) return 0;
  if (pointerY < EDGE_ZONE) return -EDGE_MAX_SPEED * Math.min(1, (EDGE_ZONE - pointerY) / EDGE_ZONE);
  const fromBottom = viewportHeight - pointerY;
  if (fromBottom < EDGE_ZONE) return EDGE_MAX_SPEED * Math.min(1, (EDGE_ZONE - fromBottom) / EDGE_ZONE);
  return 0;
}

/** 滚动偏移的合法范围: 内容不到一屏时只能是 0。 */
export function maxScroll(m: Metrics, count: number, viewportHeight: number): number {
  return Math.max(0, contentHeight(m, count) - viewportHeight);
}

/** 拖动开始后指针动了多少点才算真的在拖 (鼠标; 触屏靠长按)。 */
export const MOUSE_DRAG_SLOP = 5;
