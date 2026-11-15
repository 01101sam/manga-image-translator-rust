import { host, signal } from "nsh";

/** 顶层分区: 任务列表、PDF 阅读器、配置。 */
export type Section = "jobs" | "reader" | "config";

const SECTIONS: readonly Section[] = ["jobs", "reader", "config"];

/** 宿主可以用启动参数 `section` 指定起始分区 (UI 测试、截图用), 缺省为任务。 */
const initialSection = SECTIONS.find((s) => s === host.params.section) ?? "jobs";

export const section = signal<Section>(initialSection);

/** 查看器是盖在任务列表上的全屏层, 打开时列表保持原位; null 表示没打开。 */
export const viewerKey = signal<string | null>(null);

/** 原文还是译文: 在 Job 之间切换时保持不变。 */
export type Viewing = "translated" | "original";
export const viewing = signal<Viewing>("translated");

export function openViewer(key: string) {
  viewerKey.set(key);
}

export function closeViewer() {
  viewerKey.set(null);
}

export function toggleViewing() {
  viewing.update((v) => (v === "translated" ? "original" : "translated"));
}
