import { Image, platform, Text, View } from "nsh";
import { jobStatusLabel, shortJobId } from "../domain/job";
import { ROW_HEIGHT } from "../domain/grid";
import { mediaUrl, THUMB_WIDTH } from "../domain/media";
import { factsOf, isRemovable, type Item } from "../domain/board";
import { rejectionHint, rejectionLabel } from "../domain/rejection";
import { formatBytes, formatModified } from "../format";
import { cancelItem, highlight, itemOf, removeItems, retryItem } from "../store/jobs";
import { openMenu } from "../store/menu";
import { draggedKey, pressBlocked, startDrag } from "../store/reorder";
import { openViewer } from "../store/route";
import { session } from "../store/session";
import { Badge, Button, Icon, IconButton, type BadgeTone } from "../ui";

const desktop = platform.desktop;

/** 按住多久算长按, 进入拖动重排 (毫秒)。 */
export const LONG_PRESS_MS = 400;

export const toneOf = (item: Item): BadgeTone => {
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

export const titleOf = (item: Item): string => (item.kind === "job" ? (item.name ?? shortJobId(item.row.jobId)) : item.file.name);

/** 行里状态旁的小字: 大小和修改时间, 缺的不显示; 被拒绝的条目改说怎么办。 */
export const factsLabel = (item: Item): string => {
  if (item.kind === "pending" && item.rejected !== undefined) return rejectionHint(item.rejected) ?? "";
  const { size, modifiedAt } = factsOf(item);
  return [size === undefined ? "" : formatBytes(size), modifiedAt === undefined ? "" : formatModified(modifiedAt)].filter(Boolean).join(" · ");
};

export const statusOf = (item: Item): string => {
  switch (item.kind) {
    case "pending":
      return item.rejected === undefined ? "等待上传" : rejectionLabel(item.rejected);
    case "uploading":
      return "上传中";
    case "job":
      return jobStatusLabel(item.row);
  }
};

/**
 * 缩略图的 URL。始终是降采样的小图 (列表 w=256, 网格按格宽 256 或 512), 原图字节不会进入 JS, 也不会被整张解码。
 * 还没成为 Job 的条目 (待上传、上传中、被拒绝) Daemon 那边没有图, 用宿主导入时生成的 data URI。
 */
export function thumbUrl(item: Item, width: number = THUMB_WIDTH): string {
  if (item.kind !== "job") return item.file.thumb ?? "";
  const s = session();
  if (s.kind !== "paired") return "";
  const kind = item.row.state === "done" ? "artifact" : "source";
  return mediaUrl(s.daemon.url, item.row.jobId, kind, width, s.token);
}

/**
 * 一行。List 会把 cell 复用给别的 Item, 所以这里不存任何状态:
 * 一切都从 key 现读, 缩略图加载失败时叠在占位图标上的 Image 只是不显示。
 */
export function JobRow(props: { itemKey: () => string; /** 拖动时跟手的影子: 不当占位变淡。 */ lifted?: boolean }) {
  const item = () => itemOf(props.itemKey());
  const selected = () => highlight() === props.itemKey();
  const open = () => {
    if (pressBlocked()) return;
    const current = item();
    highlight.set(props.itemKey());
    if (current?.kind === "job" && current.row.state === "done") openViewer(props.itemKey());
  };
  return (
    <If when={item()}>
      {(current) => (
        <View
          testID="job-row"
          style={{ height: ROW_HEIGHT }}
          class={`flex-row items-center gap-1 border-b border-border pr-2 ${selected() ? "bg-accent" : ""} ${!props.lifted && draggedKey() === props.itemKey() ? "opacity-30" : ""}`}
        >
          <View
            role="button"
            accessibilityLabel={`${titleOf(current())}, ${statusOf(current())}`}
            onPress={open}
            delayLongPress={LONG_PRESS_MS}
            onLongPress={(event) => void startDrag(props.itemKey(), event.pagePosition)}
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
                <If when={factsLabel(current())}>
                  {(text) => <Text class="flex-1 truncate text-xs text-muted-foreground">{text()}</Text>}
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

/** 取消、重试、移除; 列表里已完成的行在最右边多一个箭头, 网格的单元格没有。 */
export function RowActions(props: { item: Item; itemKey: string; chevron?: boolean }) {
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
      <IconButton label={`更多: ${titleOf(props.item)}`} icon="more" tone="muted-foreground" testID="item-more" onPress={() => openMenu({ kind: "item", key: props.itemKey })} />
      <If when={(props.chevron ?? true) && props.item.kind === "job" && props.item.row.state === "done" && desktop}>
        <Icon name="chevron-right" size={16} tone="muted-foreground" />
      </If>
    </View>
  );
}