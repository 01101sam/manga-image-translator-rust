import { signal } from "nsh";
import type { ImportSource } from "../domain/import";
import { pickAndAdd } from "./jobs";
import { pickPdf } from "./reader";

/** 导入菜单是否打开 (盖在任务页上)。 */
export const importMenuOpen = signal(false);

export const openImportMenu = () => importMenuOpen.set(true);
export const closeImportMenu = () => importMenuOpen.set(false);

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
  }
}
