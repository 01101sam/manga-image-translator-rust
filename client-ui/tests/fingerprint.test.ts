import { describe, expect, test } from "nsh/test";
import { fingerprintGrid, fingerprintHash, fingerprintHex } from "../web/host.js";

/** 与 app/ImageTranslatorTests/ImageFingerprintTests.swift 里同名的生成器逐字一致: 两边对同一份像素必须给出同一个哈希。 */
function pattern(kind: "ramp" | "checker" | "lcg", width: number, height: number): Uint8Array {
  const rgba = new Uint8Array(width * height * 4);
  let state = 12345;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const index = (y * width + x) * 4;
      if (kind === "ramp") {
        rgba[index] = 255 - Math.floor((x * 255) / Math.max(width - 1, 1));
        rgba[index + 1] = Math.floor((y * 255) / Math.max(height - 1, 1));
        rgba[index + 2] = Math.floor(((x + y) * 255) / Math.max(width + height - 2, 1));
      } else if (kind === "checker") {
        const value = (Math.floor(x / 4) + Math.floor(y / 4)) % 2 === 0 ? 230 : 25;
        rgba[index] = rgba[index + 1] = rgba[index + 2] = value;
      } else {
        for (let channel = 0; channel < 3; channel++) {
          state = (Math.imul(state, 1103515245) + 12345) & 0x7fffffff;
          rgba[index + channel] = (state >> 16) & 0xff;
        }
      }
      rgba[index + 3] = 255;
    }
  }
  return rgba;
}

/** 黄金样本: Swift 与 JS 两边各自算出来必须等于这些字符串。 */
export const GOLDEN: readonly { kind: "ramp" | "checker" | "lcg"; width: number; height: number; hash: string }[] = [
  { kind: "ramp", width: 64, height: 48, hash: "673656755673aeeb567536736d676ce73673756766e7366b756766b6e6ae6767" },
  { kind: "checker", width: 100, height: 75, hash: "cc883322cc8800003322cc883322cc8833220000cc883322cc883322cc88cc88" },
  { kind: "lcg", width: 40, height: 30, hash: "724bbb128ec86b2d8c624974ad24344dc9b84ee34d59d4a8ed6662a9b6c91d68" },
  { kind: "ramp", width: 5, height: 3, hash: "1224122412241224122412241224122412241224122412241224122412241224" },
  { kind: "lcg", width: 1, height: 1, hash: "0000000000000000000000000000000000000000000000000000000000000000" },
  { kind: "checker", width: 17, height: 16, hash: "1010101010101010010101010101010110101010101010100101010101010101" },
];

describe("image fingerprint (the same algorithm as the App)", () => {
  test("the golden patterns hash to the values the Swift side produces", () => {
    for (const sample of GOLDEN) {
      expect(fingerprintHex(pattern(sample.kind, sample.width, sample.height), sample.width, sample.height)).toBe(sample.hash);
    }
  });

  test("a flat picture has no differences, and a left to right ramp sets every bit", () => {
    const flat = new Uint8Array(32 * 32 * 4).fill(200);
    expect(fingerprintHex(flat, 32, 32)).toBe("0".repeat(64));
    const ramp = new Uint8Array(17 * 16 * 4);
    for (let y = 0; y < 16; y++) for (let x = 0; x < 17; x++) ramp.fill(255 - x * 10, (y * 17 + x) * 4, (y * 17 + x) * 4 + 4);
    expect(fingerprintHash(fingerprintGrid(ramp, 17, 16))).toBe("f".repeat(64));
  });

  test("the grid averages the pixels each cell covers and every size gives 17 by 16 cells", () => {
    const two = new Uint8Array(4 * 4 * 4);
    for (let i = 0; i < 16; i++) two.set([i < 8 ? 0 : 200, i < 8 ? 0 : 200, i < 8 ? 0 : 200, 255], i * 4);
    const grid = fingerprintGrid(two, 4, 4);
    expect(grid.length).toBe(17 * 16);
    expect(grid[0]).toBe(0);
    expect(grid[16 * 17 - 1]).toBe(200);
  });
});