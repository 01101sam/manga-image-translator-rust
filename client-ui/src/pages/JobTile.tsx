import { Image, Text, View } from "nsh";
import { itemOf, highlight } from "../store/jobs";
import { openViewer } from "../store/route";
import { Badge, Icon } from "../ui";
import { RowActions, statusOf, thumbUrl, titleOf, toneOf } from "./JobRow";

/**
 * 网格里的一格: 缩略图在上, 名字和状态在下。和 JobRow 一样不存任何状态 (List 会把 cell 复用给别的 Item),
 * 点一格的行为也和点一行一样: 高亮它, 已完成的打开查看器。
 */
export function JobTile(props: { itemKey: () => string; thumbWidth: () => number }) {
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
        <View testID="job-row" class={`overflow-hidden rounded-xl border bg-card ${selected() ? "border-primary" : "border-border"}`}>
          <View role="button" accessibilityLabel={`${titleOf(current())}, ${statusOf(current())}`} onPress={open} class="active:opacity-70">
            <View class="w-full items-center justify-center overflow-hidden bg-muted" style={{ aspectRatio: 0.72 }}>
              <Icon name="image" size={28} tone="muted-foreground" />
              <If when={thumbUrl(current(), props.thumbWidth())}>
                {(src) => <Image src={src()} class="absolute size-full object-cover" />}
              </If>
            </View>
            <View class="gap-1 px-2 pt-2">
              <Text class="truncate text-xs font-medium text-foreground">{titleOf(current())}</Text>
              <Badge label={statusOf(current())} tone={toneOf(current())} />
            </View>
          </View>
          <View class="items-end px-1 pb-1">
            <RowActions item={current()} itemKey={props.itemKey()} chevron={false} />
          </View>
        </View>
      )}
    </If>
  );
}
