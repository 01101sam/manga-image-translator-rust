import { host } from "nsh";
import type { LocalFile } from "../domain/types";
import { parseFiles } from "./host";

/** 阅读器里打开的 PDF: 宿主给的引用, 字节永远不进 JS。pageCount 宿主读不到时缺省, 此时以组件的 loaded 事件为准。 */
export interface PdfFile {
  readonly id: string;
  readonly name: string;
  readonly pageCount?: number;
}

const isRecord = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null;

export function parsePdfFile(value: unknown): PdfFile | null {
  if (!isRecord(value) || typeof value.id !== "string" || typeof value.name !== "string") return null;
  const count = value.pageCount;
  return { id: value.id, name: value.name, pageCount: typeof count === "number" && Number.isInteger(count) && count > 0 ? count : undefined };
}

export function parsePdfFiles(value: unknown): PdfFile[] {
  return Array.isArray(value) ? value.flatMap((raw) => parsePdfFile(raw) ?? []) : [];
}

/** 用户可能在选择器里停很久, 取消 host.call 默认的 30 秒超时。 */
const NO_TIMEOUT = { timeout: 0 } as const;
/** 渲染一页大图在慢设备上可能超过 30 秒。 */
const RENDER_TIMEOUT = { timeout: 120_000 } as const;

export interface PdfShow {
  readonly fileId: string;
  readonly page: number;
  readonly hidden: boolean;
}

/**
 * PDF 宿主调用的类型化封装, 契约见 client-ui/HOST_PROTOCOL.md 的「PDF 阅读器」。
 * show / dispose 只有 Web 宿主提供 (pdf.js 覆盖层); App 用原生组件 it.pdf 显示。
 */
export interface PdfPort {
  pick(): Promise<PdfFile | null>;
  release(fileId: string): void;
  renderPage(fileId: string, page: number, longEdge: number): Promise<LocalFile>;
  show(request: PdfShow): Promise<number | null>;
  dispose(): void;
  onAdded(listener: (files: readonly PdfFile[]) => void): () => void;
}

export function createPdfPort(): PdfPort {
  return {
    pick: async () => {
      const answer = await host.call("pdf.pick", undefined, NO_TIMEOUT);
      return isRecord(answer) ? parsePdfFile(answer.file) : null;
    },
    release: (fileId) => host.post("pdf.release", { fileId }),
    renderPage: async (fileId, page, longEdge) => {
      const answer = await host.call("pdf.renderPage", { fileId, page, longEdge }, RENDER_TIMEOUT);
      const [file] = parseFiles([isRecord(answer) ? answer.fileRef : null]);
      if (!file) throw new Error("宿主没有返回页图");
      return file;
    },
    show: async (request) => {
      const answer = await host.call("pdf.show", request);
      const count = isRecord(answer) ? answer.pageCount : null;
      return typeof count === "number" && Number.isInteger(count) && count > 0 ? count : null;
    },
    dispose: () => host.post("pdf.dispose"),
    onAdded: (listener) => host.on("pdf.added", (payload) => listener(parsePdfFiles(payload))),
  };
}

/** 宿主端口只有一个实例; 测试通过 open() 的 host.handlers 替换宿主本身。 */
export const pdfPort: PdfPort = createPdfPort();
