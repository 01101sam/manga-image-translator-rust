import { platform, signal, type GestureResponderEvent, type ListRef, type ViewRef } from "nsh";
import { cellRect, dropMarker, edgeScrollSpeed, maxScroll, MOUSE_DRAG_SLOP, slotAt, type Metrics, type Point, type Rect } from "../domain/reorder";
import { displayIndexOf, displayOrder, moveItemTo, onRekey } from "./jobs";

/**
 * 拖动重排: 按住一项拖到新位置, 放手后只改显示顺序。
 * 被拖的项不在列表里移动: 列表原样不动, 上面盖一个跟手的影子 (ghost) 和一条指示线, 原来的那一格变淡当占位。
 * 这样虚拟化 List 复用 cell 时 (拖到边缘自动滚动, 原来的 cell 会滚出屏幕) 不影响拖动, 放手时才把新顺序一次写进 Board。
 *
 * 触屏 (iPhone、iPad、visionOS、手机浏览器) 靠长按进入: 单元格的 onLongPress 调 startDrag, 之后 List 的 scrollable 关掉,
 * 手指的后续移动由容器的 onTouchMove 收到。只靠接管触摸 (claimsTouch) 不够: iPad 模拟器实测, 长按之后第二次移动时触摸就被
 * 取消了, 同时关掉 scrollable 之后整段拖动的移动事件都收得到。
 * 桌面和 Web 的鼠标按下后移动几个点就开始拖, 容器靠 claimsTouch 接管这次触摸: nsh test 里的拖动是手指式的, 不接管会被 List 当成滚动并取消。
 */

const desktop = platform.desktop;

/** 被拖的那一项, 单元格据此变淡。只在拖动开始和结束时变, 不随指针移动。 */
export const draggedKey = signal<string | null>(null);

/** 影子: 相对列表容器的位置和大小。 */
export const ghost = signal<(Rect & { readonly key: string }) | null>(null);

/** 放手后落下的位置的指示线: 相对列表容器。 */
export const marker = signal<Rect | null>(null);

export interface ReorderBinding {
  readonly metrics: () => Metrics;
  readonly viewportHeight: () => number;
  readonly list: () => ListRef | undefined;
  readonly container: () => ViewRef | undefined;
}

let binding: ReorderBinding | undefined;
let scrollY = 0;
/** 列表容器左上角在界面里的位置: 触摸的 pageX / pageY 减去它才是容器内的坐标。 */
let origin: Point | null = null;
let pointer: Point = { x: 0, y: 0 };
/** 鼠标按下的位置: 动过几个点之后就从这里那一项开始拖。 */
let pressed: Point | null = null;
let blockPressUntil = 0;
let timer: ReturnType<typeof setInterval> | undefined;
let lastTick = 0;

interface Session {
  /** 上传成功时待上传项的 key 会换成 jobId, 拖动跟着换。 */
  key: string;
  readonly from: number;
  /** 抓住时指针在这一格里的位置。 */
  readonly grab: Point;
  target: number;
}
let session: Session | null = null;

export function bindReorder(next: ReorderBinding) {
  binding = next;
}

onRekey((from, to) => {
  if (session?.key !== from) return;
  session.key = to;
  draggedKey.set(to);
  refresh();
});

export const trackScroll = (offset: number) => {
  scrollY = offset;
  if (session) refresh();
};

/** 拖动中、以及刚拖完的一小会儿: 单元格自己的点按 (打开查看器) 不该被放手的那一下触发。 */
export const pressBlocked = (): boolean => session !== null || Date.now() < blockPressUntil;

async function measureOrigin(): Promise<void> {
  const measured = await binding?.container()?.measure();
  if (measured) origin = { x: measured.pagePosition.x, y: measured.pagePosition.y };
}

const contentPoint = (page: Point): Point => ({ x: page.x - (origin?.x ?? 0), y: page.y - (origin?.y ?? 0) + scrollY });

function refresh() {
  if (!session || !binding || !origin) return;
  const m = binding.metrics();
  const rel = { x: pointer.x - origin.x, y: pointer.y - origin.y };
  const count = displayOrder().length;
  session.target = Math.max(slotAt(m, count, { x: rel.x, y: rel.y + scrollY }), 0);
  ghost.set({ key: session.key, x: rel.x - session.grab.x, y: rel.y - session.grab.y, width: m.cellWidth, height: m.cellHeight });
  const line = dropMarker(m, session.from, session.target);
  marker.set(line && { ...line, y: line.y - scrollY });
}

function tick() {
  if (!session || !binding || !origin) return;
  const now = Date.now();
  const dt = Math.min(now - lastTick, 64);
  lastTick = now;
  // 内容被移除时被拖的项可能已经不在了: 放弃这次拖动。
  if (displayIndexOf(session.key) < 0) return cancelDrag();
  const speed = edgeScrollSpeed(pointer.y - origin.y, binding.viewportHeight());
  if (speed === 0) return;
  const limit = maxScroll(binding.metrics(), displayOrder().length, binding.viewportHeight());
  const next = Math.min(Math.max(scrollY + (speed * dt) / 1000, 0), limit);
  if (next === scrollY) return;
  binding.list()?.scrollToOffset({ offset: next, animated: false });
  scrollY = next;
  refresh();
}

/** 从 key 这一项开始拖。grabbedAt 是按住时手指或鼠标在界面里的位置 (影子相对它的偏移不变), current 是此刻的位置。 */
export async function startDrag(key: string, grabbedAt: Point, current: Point = grabbedAt): Promise<void> {
  if (session || !binding) return;
  const from = displayIndexOf(key);
  if (from < 0) return;
  pointer = current;
  if (!origin) await measureOrigin();
  if (!origin || session) return;
  const cell = cellRect(binding.metrics(), from);
  const at = contentPoint(grabbedAt);
  session = { key, from, grab: { x: at.x - cell.x, y: at.y - cell.y }, target: from };
  draggedKey.set(key);
  lastTick = Date.now();
  timer = setInterval(tick, 16);
  refresh();
}

export function endDrag() {
  const done = session;
  clear();
  if (done && done.target !== done.from) moveItemTo(done.key, done.target);
}

export function cancelDrag() {
  clear();
}

function clear() {
  if (timer !== undefined) clearInterval(timer);
  timer = undefined;
  if (session) blockPressUntil = Date.now() + 400;
  session = null;
  pressed = null;
  draggedKey.set(null);
  ghost.set(null);
  marker.set(null);
}

/**
 * 容器要不要接管这次触摸 (onMoveShouldSetResponderCapture): 触屏上只有长按之后, 桌面的鼠标按下后动过几个点。
 * 接管之后里面的 List 不会再把这次触摸当成滚动; 平时一律不接管, 所以普通的上下滑动照常滚动列表。
 */
export function claimsTouch(distance: number): boolean {
  if (session) return true;
  return desktop && pressed !== null && distance >= MOUSE_DRAG_SLOP;
}

// ---- 容器的触摸事件: 手指 (长按之后) 或鼠标的后续移动 ----

export function touchBegan(e: GestureResponderEvent) {
  void measureOrigin();
  pressed = desktop ? { x: e.pageX, y: e.pageY } : null;
}

export function touchMoved(e: GestureResponderEvent) {
  pointer = { x: e.pageX, y: e.pageY };
  if (session) {
    refresh();
    return;
  }
  if (!pressed || !origin || !binding) return;
  if (Math.hypot(pointer.x - pressed.x, pointer.y - pressed.y) < MOUSE_DRAG_SLOP) return;
  const start = pressed;
  pressed = null;
  const key = displayOrder()[slotAt(binding.metrics(), displayOrder().length, contentPoint(start))];
  if (key !== undefined) void startDrag(key, start, pointer);
}

export function touchEnded() {
  pressed = null;
  if (session) endDrag();
}

export function touchCancelled() {
  pressed = null;
  if (session) cancelDrag();
}