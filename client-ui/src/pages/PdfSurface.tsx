import { cleanup, effect, nativeComponent, platform, untracked, View } from "nsh";
import { pdfPort } from "../ports/pdf";

/** 宿主注册的 PDFKit PDFView, 契约见 HOST_PROTOCOL.md 的「PDF 阅读器」。 */
const NativePdf = nativeComponent<
  { fileId: string; page: number; hidden?: boolean },
  { loaded: { pageCount: number }; pageChanged: { page: number } }
>("it.pdf", { props: ["fileId", "page", "hidden"], events: ["loaded", "pageChanged"] });

export interface PdfSurfaceProps {
  readonly fileId: string;
  /** 从 0 起。 */
  readonly page: number;
  /** 译文叠在上面时为真: 显示端可以藏起来并停止渲染。 */
  readonly hidden: boolean;
  readonly onLoaded: (pageCount: number) => void;
  readonly onFailed: (message: string) => void;
  /** 显示端报告的换页, 包括 TSX 自己改 page 引起的回声。 */
  readonly onPageChanged: (page: number) => void;
}

/** Web 没有原生组件: 宿主 (web/host.js) 把 pdf.js 的 canvas 放进这个占位节点里。 */
function WebPdf(props: PdfSurfaceProps) {
  effect(() => {
    const request = { fileId: props.fileId, page: props.page, hidden: props.hidden };
    untracked(() => {
      pdfPort.show(request).then(
        (pageCount) => {
          if (pageCount !== null) props.onLoaded(pageCount);
        },
        (error) => props.onFailed(error instanceof Error ? error.message : String(error)),
      );
    });
  });
  cleanup(() => pdfPort.dispose());
  return <View testID="pdf-slot" class="flex-1 bg-muted" />;
}

export function PdfSurface(props: PdfSurfaceProps) {
  if (platform.os === "web") return (
      <WebPdf
        fileId={props.fileId}
        page={props.page}
        hidden={props.hidden}
        onLoaded={props.onLoaded}
        onFailed={props.onFailed}
        onPageChanged={props.onPageChanged}
      />
    );
  return (
    <NativePdf
      testID="pdf-native"
      class="flex-1 bg-muted"
      fileId={props.fileId}
      page={props.page}
      hidden={props.hidden}
      onLoaded={(event) => props.onLoaded(event.pageCount)}
      onPageChanged={(event) => props.onPageChanged(event.page)}
    />
  );
}
