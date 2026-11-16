import { fingerprintsOf } from "../domain/board";
import type { ImportSource, PickSource } from "../domain/import";
import { splitDuplicates } from "../domain/similar";
import type { LocalFile } from "../domain/types";
import { addLocalFiles, currentBoard } from "./jobs";
import { closeMenu, openMenu } from "./menu";
import { pickPdf } from "./reader";
import { section } from "./route";
import { hostPort } from "./session";
import { LONG_MS, showToast } from "./toast";

export const openImportMenu = () => openMenu({ kind: "import" });
export const closeImportMenu = closeMenu;

/** 被当作重复跳过、等用户决定要不要仍然添加的文件。宿主里的副本留着, 提示离开屏幕时放掉。 */
let skipped: LocalFile[] = [];
/** 第几次提供「仍然添加」: 新的提示盖掉旧的时, 旧提示的收尾不能把新一批也放掉。 */
let offer = 0;

function releaseSkipped() {
  const files = skipped;
  skipped = [];
  hostPort.release(files.map((file) => file.id));
}

function addSkipped() {
  const files = skipped;
  skipped = [];
  addLocalFiles(files);
}

/** 更换 Daemon 时: 没来得及决定的重复文件一起放掉。 */
export function discardSkipped() {
  offer++;
  releaseSkipped();
}

function offerSkipped(files: readonly LocalFile[]) {
  skipped = [...skipped, ...files];
  const mine = ++offer;
  showToast(`已跳过 ${skipped.length} 张重复图片`, "info", {
    ms: LONG_MS,
    action: { label: "仍然添加", run: addSkipped },
    onGone: () => {
      if (mine === offer) releaseSkipped();
    },
  });
}

/**
 * 用户导入的图片 (选择、拖放、粘贴、分享、打开方式) 进列表的入口: 和列表里已有的 (不分状态, 已移除的不算) 或同一批里更早的相似的图不加入,
 * 提示「已跳过 N 张重复图片」并带「仍然添加」。PDF 页渲染出来的页图不走这里 (阅读器要让每一页都对上自己的 Job)。
 */
export function importFiles(files: readonly LocalFile[]): void {
  const { fresh, skipped: duplicates } = splitDuplicates(files, fingerprintsOf(currentBoard()));
  if (fresh.length > 0) addLocalFiles(fresh);
  if (duplicates.length > 0) offerSkipped(duplicates);
}

/** 让宿主弹选择器, 选出来的图片走 importFiles。 */
export async function pickAndAdd(source: PickSource): Promise<void> {
  try {
    importFiles(await hostPort.pickFiles(200, source));
  } catch (error) {
    showToast(`无法选择文件: ${error instanceof Error ? error.message : String(error)}`, "error");
  }
}

/**
 * 粘贴剪贴板里的图片 (PDF 由宿主走 pdf.added, 没有可粘贴的内容时宿主发 host.notice)。
 * ⌘V 在任何分区都能按, 所以图片一到就回到任务页。
 */
export async function pasteAndAdd(): Promise<void> {
  try {
    const pasted = await hostPort.paste();
    if (pasted.length > 0) {
      section.set("jobs");
      importFiles(pasted);
    }
  } catch (error) {
    showToast(`无法粘贴: ${error instanceof Error ? error.message : String(error)}`, "error");
  }
}

/** 菜单里选了一项: 先收起菜单再让宿主去弹选择器, 免得两层同时可点。 */
export function runImport(source: ImportSource): void {
  closeImportMenu();
  switch (source) {
    case "photos":
    case "files":
    case "folder":
      void pickAndAdd(source);
      return;
    case "pdf":
      void pickPdf();
      return;
    case "paste":
      void pasteAndAdd();
      return;
  }
}
