import { effect, platform, Text, View } from "nsh";
import { currentBoard, order } from "../store/jobs";
import { closeViewer, section, viewerKey, type Section } from "../store/route";
import { engineLabel, engineState } from "../store/engine";
import { Badge, NavTabs, ToastHost, type BadgeTone, type TabItem } from "../ui";
import { ConfigPage } from "./ConfigPage";
import { JobsPage } from "./JobsPage";
import { ReaderPage } from "./ReaderPage";
import { Viewer } from "./Viewer";

const desktop = platform.desktop;

const tabs: readonly TabItem<Section>[] = [
  { value: "jobs", label: "任务", icon: "list" },
  { value: "reader", label: "阅读", icon: "doc" },
  { value: "config", label: "配置", icon: "sliders" },
];

const engineTone = (): BadgeTone => (engineState() === "running" ? "success" : engineState() === "stopped" ? "neutral" : "warning");

/** 已配对后的外壳: 触屏底部标签栏, 桌面与 Web 顶部标签栏。 */
export function Shell() {
  // 正在看的 Job 被移除时关掉查看器。
  effect(() => {
    const key = viewerKey();
    order();
    if (key !== null && currentBoard().items[key] === undefined) closeViewer();
  });
  return (
    <View class="flex-1 bg-background">
      <If when={desktop}>
        <View class="h-12 flex-row items-center gap-4 border-b border-border bg-card px-4">
          <Text class="text-sm font-semibold text-foreground">图片翻译</Text>
          <NavTabs items={tabs} value={section()} onChange={(value) => section.set(value)} />
          <View class="flex-1" />
          <Badge label={`Engine ${engineLabel(engineState())}`} tone={engineTone()} />
        </View>
      </If>
      <View class={desktop ? "flex-1" : "flex-1 pt-[env(safe-area-inset-top)]"}>
        <If when={section() === "jobs"}>
          <JobsPage />
        </If>
        <If when={section() === "reader"}>
          <ReaderPage />
        </If>
        <If when={section() === "config"}>
          <ConfigPage />
        </If>
        <If when={viewerKey() !== null && section() === "jobs"}>
          <Viewer itemKey={viewerKey() ?? ""} />
        </If>
      </View>
      <If when={!desktop}>
        <NavTabs items={tabs} value={section()} onChange={(value) => section.set(value)} />
      </If>
      <ToastHost />
    </View>
  );
}
