/**
 * Daemon 拒绝一个文件时给的原因 (HTTP 400 的 error 字段)。「无法识别图片: 不是支持的图片格式」之类的话用户读不懂,
 * 又不能说是文件坏了 (HEIC、AVIF 这类文件是好的, 只是 Daemon 解不了), 所以格式问题换成一句明确的话, 其余原样显示。
 */
const UNSUPPORTED_FORMAT = /无法识别图片|不是支持的图片格式|not supported|unsupported/i;

export const UNSUPPORTED_FORMAT_LABEL = "格式不支持";

export function rejectionLabel(message: string): string {
  return UNSUPPORTED_FORMAT.test(message) ? UNSUPPORTED_FORMAT_LABEL : message;
}

/** 状态徽章下面的补充说明: 格式不支持时告诉用户怎么办, 其余没有。 */
export function rejectionHint(message: string): string | undefined {
  return UNSUPPORTED_FORMAT.test(message) ? "Daemon 只能读 PNG、JPEG、WebP、GIF、BMP、TIFF, 请转换后重新添加" : undefined;
}
