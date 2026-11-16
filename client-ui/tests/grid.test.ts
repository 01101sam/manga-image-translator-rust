import { describe, expect, test } from "nsh/test";
import { GRID_GAP, gridColumns, MAX_COLUMNS, thumbWidthFor, tileWidth } from "../src/domain/grid";
import { gridStep } from "../src/domain/nav";

describe("grid layout", () => {
  test("phones always get the list, whatever the width of the list area", () => {
    expect(gridColumns(390, 390)).toBe(1);
    expect(gridColumns(932, 430)).toBe(1);
    expect(gridColumns(844, 390)).toBe(1);
  });

  test("a wide window turns into a grid whose columns follow the list width", () => {
    expect(gridColumns(720, 720)).toBe(3);
    expect(gridColumns(820, 820)).toBe(3);
    expect(gridColumns(1366, 1024)).toBe(6);
    expect(gridColumns(1080, 720)).toBe(5);
  });

  test("a narrow list area in a big window stays a list, and the columns are capped", () => {
    expect(gridColumns(300, 900)).toBe(1);
    expect(gridColumns(423, 900)).toBe(1);
    expect(gridColumns(424, 900)).toBe(2);
    expect(gridColumns(5000, 2000)).toBe(MAX_COLUMNS);
    expect(gridColumns(Number.NaN, 900)).toBe(1);
    expect(gridColumns(1000, Number.NaN)).toBe(1);
  });

  test("the tile width subtracts the gaps around and between the tiles", () => {
    expect(tileWidth(390, 1)).toBe(390);
    expect(tileWidth(1000, 4)).toBe((1000 - GRID_GAP * 5) / 4);
  });

  test("small tiles ask the Daemon for 256 wide previews, big ones for 512", () => {
    expect(thumbWidthFor(96)).toBe(256);
    expect(thumbWidthFor(128)).toBe(256);
    expect(thumbWidthFor(129)).toBe(512);
    expect(thumbWidthFor(240)).toBe(512);
  });
});

describe("arrow keys on the Jobs page", () => {
  test("a single column keeps up and down one item at a time and leaves left and right alone", () => {
    expect(gridStep("up", 1)).toBe(-1);
    expect(gridStep("down", 1)).toBe(1);
    expect(gridStep("left", 1)).toBeNull();
    expect(gridStep("right", 1)).toBeNull();
  });

  test("in a grid up and down move a whole row and left and right one tile", () => {
    expect(gridStep("up", 4)).toBe(-4);
    expect(gridStep("down", 4)).toBe(4);
    expect(gridStep("left", 4)).toBe(-1);
    expect(gridStep("right", 4)).toBe(1);
  });
});
