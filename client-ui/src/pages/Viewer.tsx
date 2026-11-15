import { Image, keymap, platform, signal, Text, View } from "nsh";
import { openUrl } from "nsh";
import { donePosition } from "../domain/nav";
import { mediaUrl, rawMediaUrl, VIEW_WIDTH } from "../domain/media";
import { shortJobId } from "../domain/job";
import { adjacentDoneKey, currentBoard, highlight, itemOf } from "../store/jobs";
import { closeViewer, toggleViewing, viewerKey, viewing, type Viewing } from "../store/route";
import { hostPort, session } from "../store/session";
import { showToast } from "../store/toast";
import { Button, Icon, IconButton, Segmented, Spinner } from "../ui";

const desktop = platform.desktop;

/** 每张图的加载结果按 URL 记在数据里, 切到下一张时不会沿用上一张的状态。 */
const loaded = signal<Readonly<Record<string, "ok" | "error">>>({});

function mark(url: string, result: "ok" | "error") {
  if (loaded()[url] !== result) loaded.update((m) => ({ ...m, [url]: result }));
}

const titleOf = (key: string): string => {
  const item = itemOf(key);
  if (item?.kind !== "job") return "";
  return item.name ?? shortJobId(item.row.jobId);
};

/** 盖在列表上的全屏查看器: 原文/译文切换, 在已完成的 Job 之间移动。图片用 w=2048 的降采样 JPEG。 */
export function Viewer(props: { itemKey: string }) {
  const url = (kind: "source" | "artifact") => {
    const s = session();
    return s.kind === "paired" ? mediaUrl(s.daemon.url, props.itemKey, kind, VIEW_WIDTH, s.token) : "";
  };
  const shownUrl = () => url(viewing() === "translated" ? "artifact" : "source");
  const state = () => loaded()[shownUrl()];
  const position = () => donePosition(currentBoard(), props.itemKey);
  const go = (delta: number) => {
    const next = adjacentDoneKey(props.itemKey, delta);
    if (next !== null && next !== props.itemKey) {
      viewerKey.set(next);
      highlight.set(next);
    }
  };
  keymap.bind({ left: () => go(-1), up: () => go(-1), right: () => go(1), down: () => go(1) }, { repeat: true });
  keymap.bind("space", toggleViewing);
  keymap.bind("escape", closeViewer);

  const openRaw = () => {
    const s = session();
    if (s.kind === "paired") void openUrl(rawMediaUrl(s.daemon.url, props.itemKey, viewing() === "translated" ? "artifact" : "source", s.token));
  };
  const save = async () => {
    const s = session();
    if (s.kind !== "paired") return;
    // 保存的是原始大小的文件, 不是预览用的 w=2048 降采样图; 宿主自己下载, 字节不经过 JS。
    const kind = viewing() === "translated" ? "artifact" : "source";
    const stem = (titleOf(props.itemKey) || props.itemKey).replace(/\.[^.]+$/, "");
    try {
      await hostPort.saveArtifact(rawMediaUrl(s.daemon.url, props.itemKey, kind, s.token), `${stem}${kind === "artifact" ? "-translated" : ""}`);
    } catch (error) {
      showToast(`无法保存: ${error instanceof Error ? error.message : String(error)}`, "error");
    }
  };

  return (
    <View testID="viewer" class="absolute inset-0 bg-background">
      <View class="flex-row items-center gap-2 border-b border-border px-2 pb-2 pt-[env(safe-area-inset-top)]">
        <IconButton label="返回" icon="chevron-left" testID="viewer-close" onPress={closeViewer} />
        <View class="flex-1">
          <Text class="truncate text-sm font-medium text-foreground">
            {titleOf(props.itemKey)}
          </Text>
          <If when={position()}>
            {(p) => (
              <Text testID="viewer-position" class="text-xs text-muted-foreground">
                {`${p().index} / ${p().total}`}
              </Text>
            )}
          </If>
        </View>
        <Segmented<Viewing>
          testID="viewing"
          label="查看原文或译文"
          value={viewing()}
          items={[
            { value: "translated", label: "译文" },
            { value: "original", label: "原文" },
          ]}
          onChange={(value) => viewing.set(value)}
        />
        <IconButton label="保存" icon="download" onPress={() => void save()} />
      </View>
      <View class="flex-1 items-center justify-center">
        <Image
          testID="viewer-image"
          src={shownUrl()}
          class="size-full object-contain"
          onLoad={() => mark(shownUrl(), "ok")}
          onError={() => mark(shownUrl(), "error")}
        />
        <If when={state() === undefined}>
          <View class="absolute items-center">
            <Spinner label="正在加载…" />
          </View>
        </If>
        <If when={state() === "error"}>
          <View testID="viewer-error" class="absolute items-center gap-3 rounded-(--radius) bg-card p-6">
            <Icon name="alert" size={28} tone="warning" />
            <Text class="max-w-72 text-center text-sm text-foreground">
              这个文件无法在这里预览。输出格式不是 PNG 时 (比如 HTML), 请在浏览器中打开。
            </Text>
            <Button label="在浏览器打开" icon="external" variant="secondary" size="sm" onPress={openRaw} />
          </View>
        </If>
        <View class="absolute left-2 top-1/2 -mt-5">
          <IconButton label="上一张" icon="chevron-left" tone="muted-foreground" onPress={() => go(-1)} />
        </View>
        <View class="absolute right-2 top-1/2 -mt-5">
          <IconButton label="下一张" icon="chevron-right" tone="muted-foreground" onPress={() => go(1)} />
        </View>
      </View>
      <If when={desktop}>
        <Text class="border-t border-border py-2 text-center text-xs text-muted-foreground">← → 切换 · 空格 译文/原文 · Esc 返回</Text>
      </If>
    </View>
  );
}
