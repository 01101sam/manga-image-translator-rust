import type { LocalFile } from "./domain/types";

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

const two = (n: number) => String(n).padStart(2, "0");

/** 本地时区的「2026-11-16 14:03」。不用 Intl: 嵌入 App 的 JS 引擎里它不可用。 */
export function formatModified(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())} ${two(d.getHours())}:${two(d.getMinutes())}`;
}

export const fileLabel = (file: LocalFile): string => `${file.name} · ${formatBytes(file.size)}`;