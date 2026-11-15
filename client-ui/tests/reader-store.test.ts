import { describe, expect, test } from "nsh/test";
import { newPage } from "../src/domain/reader";
import { current, nativePageChanged, pages, surfaceAttached } from "../src/store/reader";

// 原生组件的 pageChanged 无头测试发不出来, 这里直接按 HostPdf.swift 的顺序喂给 store。
describe("reader store and the host's pageChanged reports", () => {
  const open = (at: number) => {
    pages.set(Array.from({ length: 100 }, (_, i) => newPage(i)));
    current.set(at);
  };

  test("coming back to the reader at page 7: the remounted PDFView reports 0, 7, 7 and the page stays 7", () => {
    open(7);
    surfaceAttached();
    for (const reported of [0, 7, 7]) nativePageChanged(reported);
    expect(current()).toBe(7);
  });

  test("a second remount starts from scratch again", () => {
    open(7);
    surfaceAttached();
    for (const reported of [0, 7]) nativePageChanged(reported);
    surfaceAttached();
    for (const reported of [0, 7]) nativePageChanged(reported);
    expect(current()).toBe(7);
  });

  test("after the host has caught up, a swipe moves the page", () => {
    open(7);
    surfaceAttached();
    for (const reported of [0, 7, 7]) nativePageChanged(reported);
    nativePageChanged(8);
    expect(current()).toBe(8);
  });
});
