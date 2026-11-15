import { View } from "nsh";
import { Empty } from "../ui";

/** PDF 阅读器的占位页。逻辑与渲染 (App 用 PDFKit, Web 用 pdf.js) 在后续步骤接入这个分区。 */
export function ReaderPage() {
  return (
    <View testID="reader-placeholder" class="flex-1 bg-background">
      <Empty icon="doc" title="PDF 阅读器" description="打开 PDF、逐页翻译与对照阅读将在这里提供。" />
    </View>
  );
}
