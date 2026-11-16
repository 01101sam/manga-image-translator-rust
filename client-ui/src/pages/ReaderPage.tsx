import { effect, Image, keymap, platform, signal, Text, View } from "nsh";
import { mediaUrl, VIEW_WIDTH } from "../domain/media";
import { canPressSpace, pageStatus, spaceLabel, translatedJobId } from "../domain/reader";
import { engineState, runEngineAction } from "../store/engine";
import {
  closePdf,
  current,
  documentFailed,
  documentLoaded,
  exporting,
  exportPdf,
  loadError,
  nativePageChanged,
  opening,
  pages,
  pdf,
  pickPdf,
  pressSpace,
  retranslateCurrent,
  saveCurrentPage,
  step,
  surfaceAttached,
  syncJobStates,
  translateAll,
  viewing,
} from "../store/reader";
import { session } from "../store/session";
import { Banner, Button, Empty, Icon, IconButton, Spinner } from "../ui";
import { PdfSurface } from "./PdfSurface";

const desktop = platform.desktop;
/** 导出要宿主逐页写 PDF, 只有 App 宿主有; 浏览器里拼 PDF 没有低成本的办法, 网页版不提供。 */
const canExport = platform.os !== "web";

/** 当前译文图片的加载结果, 按 URL 记: 翻到别的页后不沿用上一页的状态。 */
const translated = signal<{ readonly url: string; readonly result: "ok" | "error" } | null>(null);

function mark(url: string, result: "ok" | "error") {
  if (translated()?.url !== url || translated()?.result !== result) translated.set({ url, result });
}

/** PDF 阅读器: 方向键翻页, 空格翻译本页或切换原文与译文。译文是 Daemon 的降采样 JPEG, 盖在 PDF 上面。 */
export function ReaderPage() {
  // 阅读器在屏幕上时, 把 Board 里 Job 的进度对到页上; 页面不在时回来再对一次。
  effect(syncJobStates);
  keymap.bind({ left: () => step(-1), up: () => step(-1), right: () => step(1), down: () => step(1) }, { repeat: true });
  keymap.bind("space", pressSpace);

  const page = () => pages()[current()];
  const translatedUrl = () => {
    const p = page();
    const jobId = p === undefined ? null : translatedJobId(p, viewing());
    const s = session();
    return jobId !== null && s.kind === "paired" ? mediaUrl(s.daemon.url, jobId, "artifact", VIEW_WIDTH, s.token) : null;
  };
  const translatedResult = () => {
    const url = translatedUrl();
    return url !== null && translated()?.url === url ? translated()?.result : undefined;
  };
  const status = () => {
    const p = page();
    return p === undefined ? "正在读取…" : pageStatus(p, pages().length, viewing());
  };

  return (
    <View testID="reader" class="flex-1 bg-background">
      <View class="flex-row flex-wrap items-center gap-x-3 gap-y-2 border-b border-border px-4 pb-3 pt-3">
        <View class="min-w-40 flex-1 gap-0.5">
          <Text class="text-xl font-semibold text-foreground">阅读</Text>
          <Text testID="reader-name" class="truncate text-xs text-muted-foreground">
            {pdf()?.name ?? "还没有打开 PDF"}
          </Text>
        </View>
        <If when={pdf() !== null && pages().length > 0}>
          <Button label="翻译全部" size="sm" variant="secondary" testID="reader-translate-all" onPress={translateAll} />
        </If>
        <If when={canExport && pages().some((p) => p.translation === "done")}>
          <Button label="导出" size="sm" variant="secondary" icon="download" testID="reader-export" loading={exporting()} onPress={() => void exportPdf()} />
        </If>
        <If when={pdf() !== null}>
          <Button label="换一个" size="sm" variant="secondary" icon="doc" testID="reader-open" loading={opening()} onPress={() => void pickPdf()} />
          <IconButton label="关闭 PDF" icon="x" testID="reader-close" onPress={closePdf} />
        </If>
      </View>
      <If when={engineState() === "stopped" && pdf() !== null}>
        <Banner
          testID="engine-banner"
          text="Engine 已停止, 要翻译的页会在启动后继续提交。"
          actionLabel="启动 Engine"
          onAction={() => void runEngineAction("start")}
        />
      </If>
      <If
        when={pdf()}
        else={
          <View testID="reader-empty" class="flex-1">
            <Empty
              icon="doc"
              title="打开 PDF"
              description={desktop ? "PDF 在本地显示, Daemon 只会收到你要翻译的那几页。也可以把 PDF 拖进窗口。" : "PDF 在本地显示, Daemon 只会收到你要翻译的那几页。"}
              actionLabel="打开 PDF"
              actionTestID="reader-open"
              onAction={() => void pickPdf()}
            />
          </View>
        }
      >
        {(doc) => (
          <View class="flex-1">
            <View testID="reader-canvas" class="flex-1 bg-muted">
              <PdfSurface
                fileId={doc().id}
                page={current()}
                hidden={translatedResult() === "ok"}
                onLoaded={documentLoaded}
                onFailed={documentFailed}
                onPageChanged={nativePageChanged}
                onAttach={surfaceAttached}
              />
              <If when={translatedUrl()}>
                {(url) => (
                  <Image
                    testID="reader-translated"
                    src={url()}
                    class="absolute inset-0 size-full object-contain"
                    onLoad={() => mark(url(), "ok")}
                    onError={() => mark(url(), "error")}
                  />
                )}
              </If>
              <If when={translatedUrl() !== null && translatedResult() === undefined}>
                <View class="absolute inset-x-0 top-3 items-center">
                  <Spinner label="正在加载译文…" />
                </View>
              </If>
              <If when={pages().length === 0 && loadError() === null}>
                <View class="absolute inset-0 items-center justify-center">
                  <Spinner label="正在打开…" />
                </View>
              </If>
              <If when={loadError()}>
                {(message) => (
                  <View testID="reader-error" class="absolute inset-0 items-center justify-center p-6">
                    <View class="max-w-80 items-center gap-3 rounded-(--radius) bg-card p-6">
                      <Icon name="alert" size={28} tone="warning" />
                      <Text class="text-center text-sm text-foreground">{message()}</Text>
                      <Button label="换一个 PDF" variant="secondary" size="sm" onPress={() => void pickPdf()} />
                    </View>
                  </View>
                )}
              </If>
              <If when={translatedResult() === "error"}>
                <View testID="reader-artifact-error" class="absolute inset-x-0 bottom-4 items-center px-4">
                  <View class="max-w-80 flex-row items-center gap-3 rounded-(--radius) bg-card p-3">
                    <Icon name="alert" size={20} tone="warning" />
                    <Text class="flex-1 text-sm text-foreground">译文已经不在 Daemon 上了</Text>
                    <Button label="重新翻译" size="sm" variant="secondary" onPress={retranslateCurrent} />
                  </View>
                </View>
              </If>
            </View>
            <View class="flex-row items-center gap-2 border-t border-border px-3 py-2">
              <IconButton label="上一页" icon="chevron-left" testID="reader-prev" disabled={current() <= 0} onPress={() => step(-1)} />
              <View class="flex-1 items-center">
                <Text testID="reader-status" class="text-sm text-foreground">
                  {status()}
                </Text>
                <If when={desktop}>
                  <Text class="text-xs text-muted-foreground">← → 翻页 · 空格 翻译本页 / 切换原文与译文</Text>
                </If>
              </View>
              <If when={page()?.translation === "done"}>
                <IconButton label="保存译图" icon="download" testID="reader-save" onPress={() => void saveCurrentPage()} />
              </If>
              <If when={page()}>
                {(p) => <Button label={spaceLabel(p(), viewing())} size="sm" variant="secondary" testID="reader-space" disabled={!canPressSpace(p())} onPress={pressSpace} />}
              </If>
              <IconButton label="下一页" icon="chevron-right" testID="reader-next" disabled={current() >= pages().length - 1} onPress={() => step(1)} />
            </View>
          </View>
        )}
      </If>
    </View>
  );
}