import type { ImportSource } from "../domain/import";
import { addLocalFiles, pickAndAdd } from "./jobs";
import { closeMenu, openMenu } from "./menu";
import { pickPdf } from "./reader";
import { section } from "./route";
import { hostPort } from "./session";
import { showToast } from "./toast";

export const openImportMenu = () => openMenu({ kind: "import" });
export const closeImportMenu = closeMenu;

/**
 * 粘贴剪贴板里的图片 (PDF 由宿主走 pdf.added, 没有可粘贴的内容时宿主发 host.notice)。
 * ⌘V 在任何分区都能按, 所以图片一到就回到任务页。
 */
export async function pasteAndAdd(): Promise<void> {
  try {
    const pasted = await hostPort.paste();
    if (pasted.length > 0) {
      section.set("jobs");
      addLocalFiles(pasted);
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