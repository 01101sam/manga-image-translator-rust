/**
 * 任务列表什么时候排成网格, 排几列, 缩略图取多大。纯函数, 宽度来自界面量到的列表宽与窗口尺寸。
 */

/** 窗口短边至少这么宽 (点) 才用网格: iPad 竖屏 820、visionOS 窗口 720 够, iPhone 横屏短边 430 不够, 手机始终是列表。 */
export const GRID_MIN_SHORT_EDGE = 600;
/** 单元格的目标宽度 (点)。 */
export const TILE_TARGET = 200;
export const MAX_COLUMNS = 8;
/** 网格四周和单元格之间的间距 (点)。 */
export const GRID_GAP = 8;

/** 列数; 1 表示保持一行一项的列表。 */
export function gridColumns(listWidth: number, windowShortEdge: number): number {
  if (!(windowShortEdge >= GRID_MIN_SHORT_EDGE)) return 1;
  const fits = Math.floor((listWidth - GRID_GAP) / (TILE_TARGET + GRID_GAP));
  return fits >= 2 ? Math.min(fits, MAX_COLUMNS) : 1;
}

/** 单元格的实际宽度 (点)。 */
export function tileWidth(listWidth: number, columns: number): number {
  return columns <= 1 ? listWidth : (listWidth - GRID_GAP * (columns + 1)) / columns;
}

/** 缩略图向 Daemon 要多宽的预览 (Daemon 只接受 256、512、1024、2048): 小格 256 就够, 大格按 2 倍屏需要的像素取 512。 */
export function thumbWidthFor(tilePoints: number): 256 | 512 {
  return tilePoints <= 128 ? 256 : 512;
}
/** 列表模式一行的高度 (点, 含底边线)。固定下来, 拖动重排才能由行号算出每一行在哪。 */
export const ROW_HEIGHT = 72;

/** 网格单元格里缩略图以外两块的高度 (点): 名字加状态, 以及底部的操作按钮。 */
export const TILE_INFO_HEIGHT = 52;
export const TILE_FOOTER_HEIGHT = 48;

/** 单元格缩略图的宽高比 (宽 / 高)。 */
export const TILE_ASPECT = 0.72;

/** 单元格的整体高度 (点): 缩略图按宽高比取, 再加上固定的两块。 */
export const tileHeight = (width: number): number => Math.round(width / TILE_ASPECT) + TILE_INFO_HEIGHT + TILE_FOOTER_HEIGHT;
