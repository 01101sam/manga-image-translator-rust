import { signal } from "nsh";

/** 一条短暂的提示。同一时间只显示最新一条, 3.5 秒后自动消失。 */
export interface Toast {
  readonly id: number;
  readonly text: string;
  readonly tone: "info" | "error";
}

export const toast = signal<Toast | null>(null);

let nextId = 1;
let timer: ReturnType<typeof setTimeout> | undefined;

export function showToast(text: string, tone: Toast["tone"] = "info") {
  if (timer !== undefined) clearTimeout(timer);
  const id = nextId++;
  toast.set({ id, text, tone });
  timer = setTimeout(() => {
    if (toast()?.id === id) toast.set(null);
  }, 3500);
}

export function dismissToast() {
  toast.set(null);
}
