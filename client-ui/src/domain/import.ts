/**
 * 宿主能提供的导入入口。菜单只显示宿主在 session.snapshot 里声明支持的那些: App 有相册和文件选择器,
 * 网页只有浏览器自己的文件选择。
 */
export type ImportSource = "photos" | "files" | "folder" | "pdf" | "paste";

/** 经 files.pick 选图片 (也可以带上 PDF) 的来源。 */
export type PickSource = Extract<ImportSource, "photos" | "files" | "folder">;

/** 菜单里的顺序。 */
export const IMPORT_ORDER: readonly ImportSource[] = ["photos", "files", "folder", "pdf", "paste"];

/** 宿主没有声明时的保底: 选文件和打开 PDF 任何宿主都做得到。 */
export const DEFAULT_IMPORTS: readonly ImportSource[] = ["files", "pdf"];

export const IMPORT_LABELS: Record<ImportSource, string> = {
  photos: "从相册",
  files: "从文件",
  folder: "选择文件夹",
  pdf: "打开 PDF",
  paste: "粘贴",
};

export function parseImportSources(value: unknown): ImportSource[] {
  if (!Array.isArray(value)) return [...DEFAULT_IMPORTS];
  return IMPORT_ORDER.filter((source) => value.includes(source));
}
