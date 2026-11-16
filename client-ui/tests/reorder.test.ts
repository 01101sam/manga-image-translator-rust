import { describe, expect, test } from "nsh/test";
import { GRID_GAP, ROW_HEIGHT, tileHeight, tileWidth } from "../src/domain/grid";
import {
  cellRect,
  contentHeight,
  dropMarker,
  EDGE_MAX_SPEED,
  EDGE_ZONE,
  edgeScrollSpeed,
  maxScroll,
  metricsFor,
  slotAt,
} from "../src/domain/reorder";

const near = (actual: number, expected: number) => expect(Math.abs(actual - expected) < 1e-6).toBe(true);

const list = metricsFor(390, 1);
const grid = metricsFor(1024, 4);

describe("metrics", () => {
  test("a list is one column of fixed-height rows that fill the width", () => {
    expect(list).toEqual({ columns: 1, cellWidth: 390, cellHeight: ROW_HEIGHT, gap: 0, padding: 0 });
    expect(cellRect(list, 0)).toEqual({ x: 0, y: 0, width: 390, height: ROW_HEIGHT });
    expect(cellRect(list, 3).y).toBe(3 * ROW_HEIGHT);
  });

  test("a grid pads the content and spaces the cells by the grid gap, as the List lays them out", () => {
    const width = tileWidth(1024, 4);
    expect(grid.cellWidth).toBe(width);
    expect(grid.cellHeight).toBe(tileHeight(width));
    expect(grid.padding).toBe(GRID_GAP);
    expect(cellRect(grid, 0)).toEqual({ x: GRID_GAP, y: GRID_GAP, width, height: grid.cellHeight });
    expect(cellRect(grid, 5)).toEqual({ x: GRID_GAP + (width + GRID_GAP), y: GRID_GAP + (grid.cellHeight + GRID_GAP), width, height: grid.cellHeight });
    // 最后一格的右缘离内容右缘正好是一个留白
    near(cellRect(grid, 3).x + width + GRID_GAP, 1024);
  });

  test("content height counts whole rows, the gaps between them and the padding", () => {
    expect(contentHeight(list, 0)).toBe(0);
    expect(contentHeight(list, 10)).toBe(10 * ROW_HEIGHT);
    near(contentHeight(grid, 4), 2 * GRID_GAP + grid.cellHeight);
    near(contentHeight(grid, 5), 2 * GRID_GAP + 2 * grid.cellHeight + GRID_GAP);
    expect(maxScroll(list, 20, 600)).toBe(20 * ROW_HEIGHT - 600);
    expect(maxScroll(list, 3, 600)).toBe(0);
  });
});

describe("which slot a point is on", () => {
  test("in a list the row under the point, clamped to the first and last", () => {
    expect(slotAt(list, 10, { x: 100, y: 0 })).toBe(0);
    expect(slotAt(list, 10, { x: 100, y: ROW_HEIGHT - 1 })).toBe(0);
    expect(slotAt(list, 10, { x: 100, y: ROW_HEIGHT })).toBe(1);
    expect(slotAt(list, 10, { x: 100, y: 5.5 * ROW_HEIGHT })).toBe(5);
    expect(slotAt(list, 10, { x: 100, y: -300 })).toBe(0);
    expect(slotAt(list, 10, { x: 100, y: 99_999 })).toBe(9);
    expect(slotAt(list, 0, { x: 0, y: 0 })).toBe(-1);
  });

  test("in a grid the cell by column and row; the gaps and the padding go to the nearest cell", () => {
    const step = grid.cellWidth + GRID_GAP;
    const rowStep = grid.cellHeight + GRID_GAP;
    const at = (column: number, row: number) => ({ x: GRID_GAP + column * step + 10, y: GRID_GAP + row * rowStep + 10 });
    expect(slotAt(grid, 10, at(0, 0))).toBe(0);
    expect(slotAt(grid, 10, at(3, 0))).toBe(3);
    expect(slotAt(grid, 10, at(1, 1))).toBe(5);
    expect(slotAt(grid, 10, at(2, 2))).toBe(9);
    expect(slotAt(grid, 10, { x: 5000, y: 5000 })).toBe(9);
    expect(slotAt(grid, 10, { x: 0, y: 0 })).toBe(0);
    // 最后一行没排满: 空位上的点落到最后一格
    expect(slotAt(grid, 6, at(3, 1))).toBe(5);
  });

  test("every cell's centre is on that cell", () => {
    for (const m of [list, grid]) {
      for (let index = 0; index < 23; index++) {
        const rect = cellRect(m, index);
        expect(slotAt(m, 23, { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 })).toBe(index);
      }
    }
  });
});

describe("the drop marker", () => {
  test("there is none when the item would stay where it is", () => {
    expect(dropMarker(list, 3, 3)).toBeNull();
    expect(dropMarker(grid, 0, 0)).toBeNull();
    expect(dropMarker(grid, 0, -1)).toBeNull();
  });

  test("in a list it is a line at the edge of the row the item lands on: after it going down, before it going up", () => {
    const down = dropMarker(list, 1, 4)!;
    expect(down.y + down.height / 2).toBe(5 * ROW_HEIGHT);
    expect(down.x).toBe(0);
    expect(down.width).toBe(390);
    const up = dropMarker(list, 4, 1)!;
    expect(up.y + up.height / 2).toBe(ROW_HEIGHT);
  });

  test("in a grid it is a bar in the gap beside the cell it lands on, as tall as the cell", () => {
    const cell = cellRect(grid, 6);
    const after = dropMarker(grid, 0, 6)!;
    near(after.x + after.width / 2, cell.x + cell.width + GRID_GAP / 2);
    expect(after.y).toBe(cell.y);
    expect(after.height).toBe(cell.height);
    const before = dropMarker(grid, 9, 6)!;
    near(before.x + before.width / 2, cell.x - GRID_GAP / 2);
  });
});

describe("edge auto-scroll", () => {
  const viewport = 600;

  test("nothing in the middle, faster the closer to an edge, up at the top and down at the bottom", () => {
    expect(edgeScrollSpeed(300, viewport)).toBe(0);
    expect(edgeScrollSpeed(EDGE_ZONE, viewport)).toBe(0);
    expect(edgeScrollSpeed(viewport - EDGE_ZONE, viewport)).toBe(0);
    const near = edgeScrollSpeed(viewport - EDGE_ZONE / 2, viewport);
    const edge = edgeScrollSpeed(viewport, viewport);
    expect(near).toBeGreaterThan(0);
    expect(edge).toBe(EDGE_MAX_SPEED);
    expect(near).toBeLessThan(edge);
    expect(edgeScrollSpeed(EDGE_ZONE / 2, viewport)).toBe(-near);
    expect(edgeScrollSpeed(0, viewport)).toBe(-EDGE_MAX_SPEED);
  });

  test("a finger dragged beyond the edge scrolls no faster than the maximum", () => {
    expect(edgeScrollSpeed(-200, viewport)).toBe(-EDGE_MAX_SPEED);
    expect(edgeScrollSpeed(viewport + 200, viewport)).toBe(EDGE_MAX_SPEED);
  });

  test("a viewport too short to have a middle never auto-scrolls", () => {
    expect(edgeScrollSpeed(10, EDGE_ZONE * 2)).toBe(0);
    expect(edgeScrollSpeed(0, 100)).toBe(0);
  });
});