import { batch, signal } from "nsh";
import { addLocalFiles, resetBoard } from "./jobs";
import { engineState } from "./engine";
import { loadOverrides } from "./overrides";
import { resetReader, startReader } from "./reader";
import { closeViewer, section } from "./route";
import { forgetDaemon, hostPort, startSession } from "./session";
import { LONG_MS, showToast } from "./toast";

/** 网页拖拽进行中 (宿主推送), 用来显示「松开以添加」。 */
export const dragging = signal(false);

let started = false;

/** 挂好宿主事件监听再读初始状态。只调用一次。 */
export function startApp(): void {
  if (started) return;
  started = true;
  hostPort.onFilesAdded((files) => {
    dragging.set(false);
    if (files.length > 0) {
      section.set("jobs");
      addLocalFiles(files);
    }
  });
  hostPort.onDragState((active) => dragging.set(active));
  hostPort.onNotice((text) => showToast(text, "error", LONG_MS));
  void loadOverrides();
  void startSession();
  startReader();
}

/** 更换 Daemon / 取消配对: 清掉属于旧 Daemon 的一切。 */
export function changeDaemon(): void {
  batch(() => {
    closeViewer();
    section.set("jobs");
    engineState.set("unknown");
  });
  resetBoard();
  resetReader();
  forgetDaemon();
}