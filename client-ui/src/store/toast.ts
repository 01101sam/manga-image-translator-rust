import { signal } from "nsh";

/** 提示上的一个按钮 (比如「仍然添加」)。按下就执行 run 并收起提示。 */
export interface ToastAction {
  readonly label: string;
  readonly run: () => void;
}

/** 一条短暂的提示。同一时间只显示最新一条, 过一会自动消失。 */
export interface Toast {
  readonly id: number;
  readonly text: string;
  readonly tone: "info" | "error";
  readonly action?: ToastAction;
}

export interface ToastOptions {
  /** 显示多久 (毫秒)。 */
  readonly ms?: number;
  readonly action?: ToastAction;
  /** 这条提示离开屏幕时调用一次: 到时间了、被点掉了、被新的提示盖掉了、按了按钮 (按钮的 run 先执行)。用来放掉提示替用户留着的东西。 */
  readonly onGone?: () => void;
}

export const toast = signal<Toast | null>(null);

let nextId = 1;
let timer: ReturnType<typeof setTimeout> | undefined;
let leaving: (() => void) | undefined;

/** 一般的提示 3.5 秒; 宿主的导入提示 (哪些文件没进来) 要读完再决定怎么办, 给得久一点。 */
const SHORT_MS = 3500;
export const LONG_MS = 8000;

function leave() {
  if (timer !== undefined) clearTimeout(timer);
  timer = undefined;
  const done = leaving;
  leaving = undefined;
  done?.();
}

export function showToast(text: string, tone: Toast["tone"] = "info", options: ToastOptions = {}) {
  leave();
  const id = nextId++;
  leaving = options.onGone;
  toast.set({ id, text, tone, ...(options.action === undefined ? {} : { action: options.action }) });
  timer = setTimeout(() => {
    if (toast()?.id === id) {
      toast.set(null);
      leave();
    }
  }, options.ms ?? SHORT_MS);
}

export function dismissToast() {
  toast.set(null);
  leave();
}

/** 按钮的 run 先执行, 再收起提示 (收起时 onGone 放掉提示留着的东西, run 要先把它们拿走)。 */
export function pressToastAction() {
  const action = toast()?.action;
  action?.run();
  dismissToast();
}