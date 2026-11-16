import { Image, Text, View } from "nsh";
import { TILE_FOOTER_HEIGHT, TILE_INFO_HEIGHT } from "../domain/grid";
import { itemOf, highlight } from "../store/jobs";
import { draggedKey, pressBlocked, startDrag } from "../store/reorder";
import { openViewer } from "../store/route";
import { Badge, Icon } from "../ui";
import { LONG_PRESS_MS, RowActions, statusOf, thumbUrl, titleOf, toneOf } from "./JobRow";

/**
 * 网格里的一格: 缩略图在上, 名字和状态在下。和 JobRow 一样不存任何状态 (List 会把 cell 复用给别的 Item),
 * 点一格的行为也和点一行一样: 高亮它, 已完成的打开查看器。整格的高度由调用方给定 (拖动重排要靠它算每一格的位置),
 * 缩略图占掉名字和操作按钮之外的全部。
 */
export function JobTile(props: { itemKey: () => string; thumbWidth: () => number; height: () => number; /** 拖动时跟手的影子: 不当占位变淡。 */ lifted?: boolean }) {
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
          style={{ height: props.height() }}
          class={`overflow-hidden rounded-xl border bg-card ${selected() ? "border-primary" : "border-border"} ${!props.lifted && draggedKey() === props.itemKey() ? "opacity-30" : ""}`}
        >
          <View
            role="button"
            accessibilityLabel={`${titleOf(current())}, ${statusOf(current())}`}
            onPress={open}
            delayLongPress={LONG_PRESS_MS}
            onLongPress={(event) => void startDrag(props.itemKey(), event.pagePosition)}
            class="flex-1 active:opacity-70"
          >
            <View class="w-full flex-1 items-center justify-center overflow-hidden bg-muted">
              <Icon name="image" size={28} tone="muted-foreground" />
              <If when={thumbUrl(current(), props.thumbWidth())}>
                {(src) => <Image src={src()} class="absolute size-full object-cover" />}
              </If>
            </View>
            <View class="gap-1 px-2 pt-2" style={{ height: TILE_INFO_HEIGHT }}>
              <Text class="truncate text-xs font-medium text-foreground">{titleOf(current())}</Text>
              <Badge label={statusOf(current())} tone={toneOf(current())} />
            </View>
          </View>
          <View class="items-end px-1 pb-1" style={{ height: TILE_FOOTER_HEIGHT }}>
            <RowActions item={current()} itemKey={props.itemKey()} chevron={false} />
          </View>
        </View>
      )}
    </If>
  );
}