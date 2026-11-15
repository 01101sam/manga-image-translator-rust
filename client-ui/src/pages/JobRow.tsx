import { Image, platform, Text, View } from "nsh";
import { jobStatusLabel, shortJobId } from "../domain/job";
import { mediaUrl, THUMB_WIDTH } from "../domain/media";
import { isRemovable, type Item } from "../domain/board";
import { formatBytes } from "../format";
import { cancelItem, highlight, itemOf, removeItems, retryItem } from "../store/jobs";
import { openViewer } from "../store/route";
import { session } from "../store/session";
import { Badge, Button, Icon, IconButton, type BadgeTone } from "../ui";

const desktop = platform.desktop;

const toneOf = (item: Item): BadgeTone => {
  if (item.kind !== "job") return item.kind === "pending" && item.rejected !== undefined ? "destructive" : "neutral";
  switch (item.row.state) {
    case "running":
      return "primary";
    case "done":
      return "success";
    case "failed":
      return "destructive";
    default:
      return "neutral";
  }
};

const titleOf = (item: Item): string => (item.kind === "job" ? (item.name ?? shortJobId(item.row.jobId)) : item.file.name);

const statusOf = (item: Item): string => {
  switch (item.kind) {
    case "pending":
      return item.rejected ?? "等待上传";
    case "uploading":
      return "上传中";
    case "job":
      return jobStatusLabel(item.row);
  }
};

/** 缩略图的 URL。始终是小尺寸 (w=256), 原图字节不会进入 JS, 也不会被整张解码。 */
function thumbUrl(item: Item): string {
  const s = session();
  if (item.kind !== "job" || s.kind !== "paired") return "";
  const kind = item.row.state === "done" ? "artifact" : "source";
  return mediaUrl(s.daemon.url, item.row.jobId, kind, THUMB_WIDTH, s.token);
}

/**
 * 一行。List 会把 cell 复用给别的 Item, 所以这里不存任何状态:
 * 一切都从 key 现读, 缩略图加载失败时叠在占位图标上的 Image 只是不显示。
 */
export function JobRow(props: { itemKey: () => string }) {
  const item = () => itemOf(props.itemKey());
  const selected = () => highlight() === props.itemKey();
  const open = () => {
    const current = item();
    highlight.set(props.itemKey());
    if (current?.kind === "job" && current.row.state === "done") openViewer(props.itemKey());
  };
  return (
    <If when={item()}>
      {(current) => (
        <View
          testID="job-row"
          class={`flex-row items-center gap-1 border-b border-border pr-2 ${selected() ? "bg-accent" : ""}`}
        >
          <View
            role="button"
            accessibilityLabel={`${titleOf(current())}, ${statusOf(current())}`}
            onPress={open}
            class="min-h-16 flex-1 flex-row items-center gap-3 px-4 py-2 hover:bg-accent active:opacity-70"
          >
            <View class="size-12 items-center justify-center overflow-hidden rounded-lg bg-muted">
              <Icon name="image" size={22} tone="muted-foreground" />
              <If when={thumbUrl(current())}>
                {(src) => <Image src={src()} class="absolute size-12 rounded-lg object-cover" />}
              </If>
            </View>
            <View class="flex-1 gap-1">
              <Text class="truncate text-sm font-medium text-foreground">
                {titleOf(current())}
              </Text>
              <View class="flex-row items-center gap-2">
                <Badge label={statusOf(current())} tone={toneOf(current())} />
                <If when={current().kind !== "job"}>
                  <Text class="text-xs text-muted-foreground">{formatBytes((current() as { file: { size: number } }).file.size)}</Text>
                </If>
              </View>
            </View>
          </View>
          <RowActions item={current()} itemKey={props.itemKey()} />
        </View>
      )}
    </If>
  );
}

function RowActions(props: { item: Item; itemKey: string }) {
  const cancellable = () => props.item.kind === "pending" ? props.item.rejected === undefined : props.item.kind === "job" && props.item.row.state === "queued";
  // 还在排队的项显示「取消」, 已结束的才显示移除。
  const removable = () => isRemovable(props.item) && !cancellable();
  return (
    <View class="flex-row items-center">
      <If when={props.item.kind === "pending" && props.item.rejected !== undefined}>
        <Button label="重试" size="sm" variant="ghost" onPress={() => retryItem(props.itemKey)} />
      </If>
      <If when={cancellable()}>
        <Button
          label="取消"
          size="sm"
          variant="ghost"
          testID="job-cancel"
          disabled={props.item.kind === "job" && props.item.row.cancelPending}
          onPress={() => void cancelItem(props.itemKey)}
        />
      </If>
      <If when={removable()}>
        <IconButton label="移除" icon="x" tone="muted-foreground" onPress={() => removeItems([props.itemKey])} />
      </If>
      <If when={props.item.kind === "job" && props.item.row.state === "done" && desktop}>
        <Icon name="chevron-right" size={16} tone="muted-foreground" />
      </If>
    </View>
  );
}
