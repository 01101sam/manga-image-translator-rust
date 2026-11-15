/**
 * 图片 URL 的构造。图片字节不进 JS: NSH 的 Image 与浏览器的 <img> 直接加载这些 URL,
 * Token 放在查询参数里 (它们发不出 Authorization 头)。
 */
export type MediaKind = "source" | "artifact";

/** Daemon 允许的 `w` 取值, 见 `preview::WIDTHS`。列表缩略图永远用小的, 查看器用大的。 */
export const THUMB_WIDTH = 256;
export const VIEW_WIDTH = 2048;

export function mediaUrl(baseUrl: string, jobId: string, kind: MediaKind, width: number, token: string): string {
  return `${baseUrl}/jobs/${encodeURIComponent(jobId)}/${kind}?w=${width}&token=${encodeURIComponent(token)}`;
}

/** 在浏览器里直接打开原始大小的文件 (不带 w), 用于 HTML 等不是图片的 Artifact。 */
export function rawMediaUrl(baseUrl: string, jobId: string, kind: MediaKind, token: string): string {
  return `${baseUrl}/jobs/${encodeURIComponent(jobId)}/${kind}?token=${encodeURIComponent(token)}`;
}
