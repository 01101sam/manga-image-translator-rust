import type { LocalFile } from "./types";

/**
 * 相似图片: 同一张图换了分辨率、重新压缩之后还能认出来。宿主在导入时算每张图的 256 位 dHash (App 的 ImageFingerprint.swift 与 web/host.js 同一个算法),
 * 这里只比较。两张图相似 = 汉明距离不超过 SIMILAR_DISTANCE, 并且宽高比相差不到 ASPECT_TOLERANCE。
 *
 * 阈值来自 scripts/fingerprint-study 的实测 (数字和过程在 client-ui/DESIGN.md): 同一张图缩放 0.25 到 2 倍、JPEG 重压缩的变体最远 12 位 (页和近空白页一样),
 * 不同的页最近 45 位, 一页和近空白页最近 24 位; 取 20, 离同图的最远 8 位, 离最近的异图 4 位。
 */
export const SIMILAR_DISTANCE = 20;
export const ASPECT_TOLERANCE = 0.03;

const HASH_PATTERN = /^[0-9a-f]{64}$/;

/** 一张图的指纹: 256 位拆成 8 个 32 位整数 (比较时按字处理, 不逐位), 加上宽高比 (没有时不参与判断)。 */
export interface Fingerprint {
  readonly words: readonly number[];
  readonly aspect?: number;
}

export function parseHash(hash: unknown): readonly number[] | undefined {
  if (typeof hash !== "string" || !HASH_PATTERN.test(hash)) return undefined;
  return Array.from({ length: 8 }, (_, index) => parseInt(hash.slice(index * 8, index * 8 + 8), 16) | 0);
}

export function fingerprintOf(hash: unknown, aspect: number | undefined): Fingerprint | undefined {
  const words = parseHash(hash);
  if (words === undefined) return undefined;
  return aspect !== undefined && Number.isFinite(aspect) && aspect > 0 ? { words, aspect } : { words };
}

const popcount = (value: number): number => {
  let x = value - ((value >>> 1) & 0x55555555);
  x = (x & 0x33333333) + ((x >>> 2) & 0x33333333);
  x = (x + (x >>> 4)) & 0x0f0f0f0f;
  return Math.imul(x, 0x01010101) >>> 24;
};

/** 两个指纹相差的位数; 超过 limit 就提前停 (大多数不同的图一两个字就超了), 返回值此时大于 limit。 */
export function distance(a: Fingerprint, b: Fingerprint, limit: number = 256): number {
  let total = 0;
  for (let index = 0; index < 8; index++) {
    total += popcount(a.words[index]! ^ b.words[index]!);
    if (total > limit) return total;
  }
  return total;
}

/** 宽高比相差的比例; 有一边没有宽高比就认为没有差别 (不拿它否决)。 */
export function aspectDifference(a: Fingerprint, b: Fingerprint): number {
  if (a.aspect === undefined || b.aspect === undefined) return 0;
  return Math.abs(a.aspect - b.aspect) / Math.max(a.aspect, b.aspect);
}

export const isSimilar = (a: Fingerprint, b: Fingerprint): boolean =>
  distance(a, b, SIMILAR_DISTANCE) <= SIMILAR_DISTANCE && aspectDifference(a, b) < ASPECT_TOLERANCE;

export interface Split {
  /** 照常加入的。没有指纹的 (宿主没算出来、格式解不开) 也在这里: 认不出就不拦。 */
  readonly fresh: LocalFile[];
  /** 和已有的、或同一批里更早的相似的图。 */
  readonly skipped: LocalFile[];
}

/** 一批新导入的文件里, 哪些和 known (列表里已有的所有条目, 不分状态) 或这一批里更早的某张相似。顺序不变。 */
export function splitDuplicates(batch: readonly LocalFile[], known: readonly Fingerprint[]): Split {
  const seen = [...known];
  const fresh: LocalFile[] = [];
  const skipped: LocalFile[] = [];
  for (const file of batch) {
    const print = fingerprintOf(file.hash, file.aspect);
    if (print === undefined) {
      fresh.push(file);
    } else if (seen.some((other) => isSimilar(print, other))) {
      skipped.push(file);
    } else {
      seen.push(print);
      fresh.push(file);
    }
  }
  return { fresh, skipped };
}