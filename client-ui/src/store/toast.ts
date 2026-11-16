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
/** 当前提示的显示时长, 合并时取两条里长的。 */
let shownMs = 0;

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

/**
 * 同一时间只显示一条提示, 后来的盖掉前面的; 例外: 一条带按钮的提示 (比如「已跳过 N 张重复图片」+「仍然添加」) 和一条不带按钮的提示
 * 碰在一起时合成一条 (文字换行连起来, 按钮留着), 因为盖掉带按钮的那条会让用户没机会按, 它替用户留着的文件也被放掉了。
 * 宿主在同一次导入里先发 files.added 再发 host.notice (有文件没能读进来), 两条几乎同时到。两条都带按钮时仍然是后来的盖掉前面的。
 */
export function showToast(text: string, tone: Toast["tone"] = "info", options: ToastOptions = {}) {
  const current = toast();
  const merge = current !== null && (current.action === undefined) !== (options.action === undefined);
  const gone = merge ? chain(leaving, options.onGone) : options.onGone;
  if (merge) {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    leaving = undefined;
  } else {
    leave();
  }
  const id = nextId++;
  leaving = gone;
  const action = merge ? (current.action ?? options.action) : options.action;
  const ms = Math.max(options.ms ?? SHORT_MS, merge ? shownMs : 0);
  shownMs = ms;
  toast.set({
    id,
    text: merge ? `${current.text}\n${text}` : text,
    tone: merge && current.tone === "error" ? "error" : tone,
    ...(action === undefined ? {} : { action }),
  });
  timer = setTimeout(() => {
    if (toast()?.id === id) {
      toast.set(null);
      leave();
    }
  }, ms);
}

function chain(first: (() => void) | undefined, second: (() => void) | undefined): (() => void) | undefined {
  if (first === undefined || second === undefined) return first ?? second;
  return () => {
    first();
    second();
  };
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