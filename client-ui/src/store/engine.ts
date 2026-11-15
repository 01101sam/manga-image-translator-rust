import { signal } from "nsh";
import type { EngineState } from "../domain/types";
import { api, handleError } from "./session";
import { showToast } from "./toast";

/** unknown: 还没取到, 或 Daemon 不可达。 */
export const engineState = signal<EngineState | "unknown">("unknown");
export const engineBusy = signal(false);

/** Daemon 的队列容量, 上传调度据此限制在途数量。没取到之前不上传: 猜小了浪费吞吐, 猜大了会吃 429。 */
export const queueCapacity = signal<number | null>(null);

/** Daemon 的 Engine 停着时提交会得到 503, 所以上传调度据此暂停。 */
export const engineAcceptsJobs = (): boolean => {
  const state = engineState();
  return state !== "stopped" && state !== "draining";
};

export const engineLabel = (state: EngineState | "unknown"): string => {
  switch (state) {
    case "stopped":
      return "已停止";
    case "starting":
      return "启动中";
    case "running":
      return "运行中";
    case "draining":
      return "排空中";
    case "unknown":
      return "未知";
  }
};

let refreshing = false;

/** 轮询用: 上一次还没回来就跳过, 启动期间 Daemon 会占着 Engine 锁, 不能叠请求。 */
export async function refreshEngine(): Promise<void> {
  const client = api();
  if (!client || refreshing || engineBusy()) return;
  refreshing = true;
  try {
    const status = await client.engineStatus();
    engineState.set(status.state);
    queueCapacity.set(status.queueCapacity);
  } catch (error) {
    if (!handleError(error)) engineState.set("unknown");
  } finally {
    refreshing = false;
  }
}

const optimistic = { start: "starting", stop: "draining", restart: "starting" } as const;

export async function runEngineAction(action: "start" | "stop" | "restart"): Promise<void> {
  const client = api();
  if (!client || engineBusy()) return;
  engineBusy.set(true);
  // 启动要等模型加载, 期间 Daemon 不应答状态查询, 先显示目标状态。
  engineState.set(optimistic[action]);
  try {
    engineState.set(await client.engineAction(action));
  } catch (error) {
    if (!handleError(error)) showToast(error instanceof Error ? error.message : String(error), "error");
    engineBusy.set(false);
    await refreshEngine();
    return;
  }
  engineBusy.set(false);
}
