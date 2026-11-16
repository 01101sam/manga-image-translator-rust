import { signal } from "nsh";

/** 一条短暂的提示。同一时间只显示最新一条, 过一会自动消失。 */
export interface Toast {
  readonly id: number;
  readonly text: string;
  readonly tone: "info" | "error";
}

export const toast = signal<Toast | null>(null);

let nextId = 1;
let timer: ReturnType<typeof setTimeout> | undefined;

/** 一般的提示 3.5 秒; 宿主的导入提示 (哪些文件没进来) 要读完再决定怎么办, 给得久一点。 */
const SHORT_MS = 3500;
export const LONG_MS = 8000;

export function showToast(text: string, tone: Toast["tone"] = "info", ms: number = SHORT_MS) {
  if (timer !== undefined) clearTimeout(timer);
  const id = nextId++;
  toast.set({ id, text, tone });
  timer = setTimeout(() => {
    if (toast()?.id === id) toast.set(null);
  }, ms);
}

export function dismissToast() {
  toast.set(null);
}