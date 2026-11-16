import { keymap, List, platform, signal, Text, View, windowSize, type ListRef } from "nsh";
import { LANGUAGES } from "../domain/config";
import { canOpenJob } from "../domain/job";
import { GRID_GAP, gridColumns, thumbWidthFor, tileWidth } from "../domain/grid";
import { gridStep, type Arrow } from "../domain/nav";
import type { Counts } from "../domain/board";
import { dragging } from "../store/app";
import { engineState, runEngineAction } from "../store/engine";
import {
  clearFinished,
  counts,
  currentBoard,
  highlight,
  moveSelection,
  notice,
  order,
  pickAndAdd,
  removeItems,
} from "../store/jobs";
import { overrides, setOverride } from "../store/overrides";
import { openViewer, viewerKey } from "../store/route";
import { Banner, Button, Card, Empty, Icon, Meter, SelectField } from "../ui";
import { JobRow } from "./JobRow";
import { JobTile } from "./JobTile";

const desktop = platform.desktop;
const optionsOpen = signal(false);

function summary(c: Counts): string {
  const parts: string[] = [];
  if (c.pending + c.uploading > 0) parts.push(`待上传 ${c.pending + c.uploading}`);
  if (c.queued > 0) parts.push(`排队 ${c.queued}`);
  if (c.running > 0) parts.push(`翻译中 ${c.running}`);
  if (c.failed > 0) parts.push(`失败 ${c.failed}`);
  return parts.length > 0 ? parts.join(" · ") : "全部完成";
}

/** 已处理的比例: 完成、失败、取消都算处理完。 */
const progressOf = (c: Counts): number => (c.total === 0 ? 0 : (c.done + c.failed + c.cancelled) / c.total);

export function JobsPage() {
  let list: ListRef | undefined;
  // 当前屏幕上可见的行范围, 键盘高亮移出范围时才滚动, 否则列表原地不动。
  let visible = { first: 0, last: 0 };
  // 列表区域的宽度 (点), 布局之后由 onLayout 更新; 起始值取窗口宽度, 第一帧就接近对。
  const listWidth = signal(windowSize().width);
  // iPad、visionOS 和桌面浏览器的宽窗口排成网格, 手机保持一行一项。
  const columns = () => gridColumns(listWidth(), Math.min(windowSize().width, windowSize().height));
  const thumbWidth = () => thumbWidthFor(tileWidth(listWidth(), columns()));

  const select = (delta: number) => {
    if (viewerKey() !== null) return;
    const index = moveSelection(delta);
    if (index === null || !list) return;
    if (index < visible.first) list.scrollToIndex({ index, viewPosition: 0, animated: false });
    else if (index > visible.last) list.scrollToIndex({ index, viewPosition: 1, animated: false });
  };
  const openHighlighted = () => {
    const key = highlight();
    if (viewerKey() !== null || key === null) return;
    const item = currentBoard().items[key];
    if (item?.kind === "job" && canOpenJob(item.row.state)) openViewer(key);
  };
  const arrow = (which: Arrow) => {
    const step = gridStep(which, columns());
    if (step !== null) select(step);
  };
  keymap.bind("up", () => arrow("up"), { repeat: true });
  keymap.bind("down", () => arrow("down"), { repeat: true });
  keymap.bind("left", () => arrow("left"), { repeat: true });
  keymap.bind("right", () => arrow("right"), { repeat: true });
  keymap.bind("enter", openHighlighted);
  keymap.bind("space", openHighlighted);
  // Mac 键盘上的「删除」键是 backspace。
  const removeHighlighted = () => {
    const key = highlight();
    if (viewerKey() === null && key !== null) removeItems([key]);
  };
  keymap.bind("delete", removeHighlighted);
  keymap.bind("backspace", removeHighlighted);

  return (
    <View class="flex-1 bg-background">
      <View class="gap-3 border-b border-border px-4 pb-3 pt-3">
        <View class="flex-row flex-wrap items-center justify-between gap-x-3 gap-y-2">
          <View class="min-w-40 flex-1 gap-0.5">
            <Text class="text-xl font-semibold text-foreground">任务</Text>
            <Text testID="jobs-summary" class="text-xs text-muted-foreground">
              {counts().total === 0 ? "还没有任务" : `共 ${counts().total} 张 · ${summary(counts())}`}
            </Text>
          </View>
          <Button label="选项" size="sm" variant="ghost" icon="sliders" onPress={() => optionsOpen.update((v) => !v)} />
          <If when={counts().done + counts().failed + counts().cancelled > 0}>
            <Button label="清除已结束" size="sm" variant="secondary" testID="clear-finished" onPress={clearFinished} />
          </If>
          <Button label="添加图片" size="sm" icon="plus" testID="add-images" onPress={() => void pickAndAdd()} />
        </View>
        <If when={counts().total > 0}>
          <Meter value={progressOf(counts())} testID="jobs-progress" />
        </If>
        <If when={optionsOpen()}>
          <Card>
            <Text class="text-xs text-muted-foreground">只影响之后提交的图片; 留空则使用 Daemon 配置里的值。</Text>
            <View class={desktop ? "flex-row gap-3" : "gap-3"}>
              <View class="flex-1">
                <SelectField
                  label="目标语言"
                  value={overrides().target_lang ?? null}
                  options={LANGUAGES}
                  emptyLabel="跟随配置"
                  onChange={(value) => setOverride("target_lang", value)}
                />
              </View>
              <View class="flex-1">
                <SelectField
                  label="检测器"
                  value={overrides().detector ?? null}
                  options={[{ value: "DBNet", label: "DBNet" }, { value: "Ctd", label: "CTD" }]}
                  emptyLabel="跟随配置"
                  onChange={(value) => setOverride("detector", value)}
                />
              </View>
              <View class="flex-1">
                <SelectField
                  label="翻译方式"
                  value={overrides().translator ?? null}
                  options={[{ value: "Llm", label: "LLM 翻译" }, { value: "Original", label: "按原文输出" }, { value: "None", label: "不输出文字" }]}
                  emptyLabel="跟随配置"
                  onChange={(value) => setOverride("translator", value)}
                />
              </View>
            </View>
          </Card>
        </If>
      </View>
      <If when={engineState() === "stopped"}>
        <Banner
          testID="engine-banner"
          text="Engine 已停止, 等待上传的图片会在启动后继续提交。"
          actionLabel="启动 Engine"
          onAction={() => void runEngineAction("start")}
        />
      </If>
      <If when={notice()}>{(text) => <Banner testID="net-banner" tone="destructive" text={text()} />}</If>
      <View class="flex-1" onLayout={(event) => listWidth.set(event.bounds.size.width)}>
        <If
          when={order().length > 0}
          else={
            <Empty
              icon="image"
              title="拖入图片, 或点击添加"
              description={desktop ? "支持 PNG、JPEG、WebP、BMP。可以一次拖入很多张, 会按顺序提交。" : "可以一次选很多张, 会按顺序提交。"}
              actionLabel="添加图片"
              onAction={() => void pickAndAdd()}
            />
          }
        >
          <List
            testID={columns() > 1 ? "job-grid" : "job-list"}
            ref={(node) => (list = node)}
            each={order()}
            keyExtractor={(key) => key}
            numColumns={columns()}
            estimatedItemSize={columns() > 1 ? tileWidth(listWidth(), columns()) / 0.72 + 80 : 72}
            contentStyle={columns() > 1 ? { padding: GRID_GAP, gap: GRID_GAP } : {}}
            onViewableItemsChange={({ viewableItems }) => {
              const indexes = viewableItems.filter((t) => t.isViewable).map((t) => t.index);
              if (indexes.length > 0) visible = { first: Math.min(...indexes), last: Math.max(...indexes) };
            }}
            class="flex-1"
          >
            {(key) => (
              <If when={columns() > 1} else={<JobRow itemKey={key} />}>
                <JobTile itemKey={key} thumbWidth={thumbWidth} />
              </If>
            )}
          </List>
        </If>
        <If when={dragging()}>
          <View testID="drop-overlay" class="absolute inset-3 items-center justify-center gap-2 rounded-2xl border-2 border-dashed border-primary bg-background/90">
            <Icon name="download" size={32} tone="primary" />
            <Text class="text-base font-medium text-primary">松开以添加图片</Text>
          </View>
        </If>
      </View>
    </View>
  );
}
