import { signal, Storage } from "nsh";
import type { JobOverrides } from "../domain/types";

/** 本次提交覆盖 Config 的选项。undefined 表示跟随 Daemon 的 Config。 */
export const overrides = signal<JobOverrides>({});

const KEY = "submit.overrides";

export async function loadOverrides(): Promise<void> {
  try {
    const raw = await Storage.getItem(KEY);
    if (raw) overrides.set(JSON.parse(raw) as JobOverrides);
  } catch {
    // 存的内容坏了就当没有。
  }
}

export function setOverride(key: keyof JobOverrides, value: string | null) {
  const next: Record<string, string> = { ...overrides() };
  if (value === null) delete next[key];
  else next[key] = value;
  overrides.set(next);
  void Storage.setItem(KEY, JSON.stringify(next));
}
