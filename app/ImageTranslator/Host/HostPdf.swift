import CoreGraphics
import ImageIO
import NativeShell
import PDFKit
import UIKit
import UniformTypeIdentifiers

/// JS 的数字到 Swift 可能是 Int、Double 或 NSNumber。
func hostInt(_ value: Any?) -> Int? {
    switch value {
    case let value as Int: return value
    case let value as Double: return value.isFinite ? Int(value) : nil
    case let value as NSNumber: return value.intValue
    default: return nil
    }
}

/// 原生组件 it.pdf (HOST_PROTOCOL.md 的「PDF 阅读器」一节): PDFKit 的 PDFView 只渲染当前页。
@MainActor
final class PdfComponent: NshComponent {
    private let pdfView = PDFView()
    private let context: NshComponentContext
    private let files: ManagedFiles
    private var loadedID: String?
    private var wantedPage = 0

    var view: UIView { pdfView }

    init(context: NshComponentContext, files: ManagedFiles) {
        self.context = context
        self.files = files
        pdfView.displayMode = .singlePage
        pdfView.displayDirection = .horizontal
        pdfView.autoScales = true
        pdfView.usePageViewController(true)
        pdfView.pageShadowsEnabled = false
        // 默认的页间距会让页面四周多出一圈边, 比译文层 (contain 铺满整个区域) 小一点, 原文和译文切换时会跳一下。
        pdfView.displaysPageBreaks = false
        pdfView.accessibilityIdentifier = "pdf-native-view"
        NotificationCenter.default.addObserver(self, selector: #selector(pageChanged), name: .PDFViewPageChanged, object: pdfView)
    }

    func setProp(_ name: String, _ value: Any?) {
        switch name {
        case "fileId":
            guard let id = value as? String, id != loadedID else { return }
            load(id)
        case "page":
            guard let page = hostInt(value) else { return }
            wantedPage = page
            show(page: page)
        case "hidden":
            pdfView.isHidden = (value as? Bool) ?? false
        default:
            break
        }
    }

    func command(_ name: String, _ args: Any?) {
        switch name {
        case "next": pdfView.goToNextPage(nil)
        case "prev": pdfView.goToPreviousPage(nil)
        case "goto":
            let number = hostInt(args) ?? hostInt((args as? [String: Any])?["page"])
            if let number { show(page: number) }
        default:
            break
        }
    }

    private func load(_ id: String) {
        loadedID = id
        guard let url = files.pdfURL(for: id), let document = PDFDocument(url: url) else {
            pdfView.document = nil
            return
        }
        // 设置 document 会触发 PDFViewPageChanged 把 wantedPage 写成 0, 先把 TSX 想要的页存下来。
        let target = wantedPage
        pdfView.document = document
        context.emit("loaded", ["pageCount": document.pageCount])
        show(page: target)
        context.emit("pageChanged", ["page": currentIndex ?? 0])
    }

    private var currentIndex: Int? {
        guard let document = pdfView.document, let page = pdfView.currentPage else { return nil }
        return document.index(for: page)
    }

    private func show(page index: Int) {
        guard let document = pdfView.document, index >= 0, index < document.pageCount,
              index != currentIndex, let page = document.page(at: index)
        else { return }
        pdfView.go(to: page)
    }

    @objc private func pageChanged() {
        if let index = currentIndex {
            wantedPage = index
            context.emit("pageChanged", ["page": index])
        }
    }
}

/// pdf.renderPage: 用 CGContext 把一页按长边画成 PNG 文件, 不经过 UIImage 也不生成 PNG 的 Data。
enum PdfRenderer {
    static let defaultLongEdge = 3072
    static let longEdgeRange = 256...6144

    static func clampedLongEdge(_ requested: Int?) -> Int {
        min(max(requested ?? defaultLongEdge, longEdgeRange.lowerBound), longEdgeRange.upperBound)
    }

    /// 页面显示框 (cropBox) 在 /Rotate 之后的尺寸。
    static func displaySize(of page: CGPDFPage) -> CGSize {
        let box = page.getBoxRect(.cropBox)
        return page.rotationAngle % 180 == 0 ? box.size : CGSize(width: box.height, height: box.width)
    }

    static func pixelSize(for page: CGSize, longEdge: Int) -> (width: Int, height: Int) {
        let scale = CGFloat(longEdge) / max(page.width, page.height, 1)
        return (max(1, Int((page.width * scale).rounded())), max(1, Int((page.height * scale).rounded())))
    }

    static func open(_ url: URL) throws -> CGPDFDocument {
        guard let document = CGPDFDocument(url as CFURL) else { throw HostCallError("打不开这个 PDF") }
        if document.isEncrypted, !document.isUnlocked, !document.unlockWithPassword("") {
            throw HostCallError("这个 PDF 有密码, 暂不支持")
        }
        return document
    }

    static func render(document url: URL, page index: Int, longEdge: Int, to output: URL) throws {
        try autoreleasepool {
            let document = try open(url)
            guard index >= 0, index < document.numberOfPages, let page = document.page(at: index + 1) else {
                throw HostCallError("页码超出范围: 第 \(index + 1) 页, 共 \(document.numberOfPages) 页")
            }
            let pageSize = displaySize(of: page)
            let size = pixelSize(for: pageSize, longEdge: longEdge)
            guard let context = CGContext(
                data: nil, width: size.width, height: size.height, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
            ) else { throw HostCallError("内存不足, 渲染不了这一页") }
            let canvas = CGRect(x: 0, y: 0, width: size.width, height: size.height)
            context.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
            context.fill(canvas)
            context.interpolationQuality = .high
            // getDrawingTransform 只缩小不放大: 直接对画布求变换, 页面会按 72dpi 的点尺寸缩在画布中间。
            // 先把点放大到像素, 再对「页面自己大小的矩形」求变换, 它只负责 cropBox 偏移和 /Rotate。
            context.scaleBy(x: CGFloat(size.width) / pageSize.width, y: CGFloat(size.height) / pageSize.height)
            context.concatenate(page.getDrawingTransform(.cropBox, rect: CGRect(origin: .zero, size: pageSize), rotate: 0, preserveAspectRatio: true))
            context.drawPDFPage(page)
            guard let image = context.makeImage(),
                  let destination = CGImageDestinationCreateWithURL(output as CFURL, UTType.png.identifier as CFString, 1, nil)
            else { throw HostCallError("写不出 PNG") }
            CGImageDestinationAddImage(destination, image, nil)
            guard CGImageDestinationFinalize(destination) else { throw HostCallError("写 PNG 失败") }
        }
    }
}
/// pdf.export: 译文页和没翻译的原页按页序拼成一个新 PDF (HOST_PROTOCOL.md 的 pdf.export)。
/// 逐页处理: 一页的译文图下载到临时文件、降采样、画进 PDF 页、删掉, 才轮到下一页, 同一时刻最多一页的位图在内存里。
enum PdfExporter {
    /// 译文图降采样到页面点尺寸的这么多倍: 2 倍在 iPad 上放大看仍清晰, 文件大小按它的平方增长。
    static let pixelsPerPoint: CGFloat = 2
    static let jpegQuality: CGFloat = 0.85

    struct Outcome: Equatable, Sendable {
        /// 用了译文图的页数。
        var translated = 0
        /// 给了译文地址、但取不到或不是图片 (比如 Daemon 重启后 Artifact 没了) 而退回原页的页数。
        var missing = 0
    }

    /// artifact 对页码 (从 0 起) 返回译文图的本地文件, 返回 nil 表示这页用原页, 抛错表示这页的译文取不到 (记入 missing, 也用原页)。
    /// release 在这一页用完后调用, 由调用方删临时文件。
    static func compose(
        source: URL, output: URL,
        artifact: (Int) async throws -> URL?,
        release: (URL) -> Void = { _ in }
    ) async throws -> Outcome {
        let document = try PdfRenderer.open(source)
        var firstBox = CGRect(x: 0, y: 0, width: 612, height: 792)
        guard let context = CGContext(output as CFURL, mediaBox: &firstBox, nil) else { throw HostCallError("写不出 PDF") }
        var outcome = Outcome()
        for index in 0..<document.numberOfPages {
            guard let page = document.page(at: index + 1) else { throw HostCallError("读不出第 \(index + 1) 页") }
            var file: URL?
            var asked = false
            do {
                file = try await artifact(index)
                asked = file != nil
            } catch {
                outcome.missing += 1
            }
            if appendPage(page, translated: file, to: context) {
                outcome.translated += 1
            } else if asked {
                outcome.missing += 1
            }
            if let file { release(file) }
        }
        context.closePDF()
        return outcome
    }

    /// 写一页。有译文图且读得出就画译文图, 否则画原页 (保留矢量)。返回是否用了译文图。
    private static func appendPage(_ page: CGPDFPage, translated file: URL?, to context: CGContext) -> Bool {
        autoreleasepool {
            let size = PdfRenderer.displaySize(of: page)
            var box = CGRect(origin: .zero, size: size)
            context.beginPDFPage([kCGPDFContextMediaBox as String: Data(bytes: &box, count: MemoryLayout<CGRect>.size)] as CFDictionary)
            defer { context.endPDFPage() }
            if let file, let image = downsampledJPEG(at: file, maxPixel: Int((max(size.width, size.height) * pixelsPerPoint).rounded())) {
                context.draw(image, in: fit(CGSize(width: image.width, height: image.height), in: box))
                return true
            }
            context.saveGState()
            context.concatenate(page.getDrawingTransform(.cropBox, rect: box, rotate: 0, preserveAspectRatio: true))
            context.drawPDFPage(page)
            context.restoreGState()
            return false
        }
    }

    static func fit(_ content: CGSize, in box: CGRect) -> CGRect {
        guard content.width > 0, content.height > 0 else { return box }
        let scale = min(box.width / content.width, box.height / content.height)
        let size = CGSize(width: content.width * scale, height: content.height * scale)
        return CGRect(x: box.midX - size.width / 2, y: box.midY - size.height / 2, width: size.width, height: size.height)
    }

    /// 读出降采样后的位图, 再编成 JPEG 并作为 JPEG 数据源的图返回: CoreGraphics 对这种图把 DCT 流原样嵌进 PDF,
    /// 比 Flate 压缩的位图小得多, 也不会在写完整个文件前一直留着每页的位图。图读不出来时返回 nil。
    static func downsampledJPEG(at url: URL, maxPixel: Int) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: max(maxPixel, 1),
        ]
        guard let bitmap = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        let data = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(data, UTType.jpeg.identifier as CFString, 1, nil) else { return bitmap }
        CGImageDestinationAddImage(destination, bitmap, [
            kCGImageDestinationLossyCompressionQuality: jpegQuality,
            kCGImageDestinationBackgroundColor: CGColor(red: 1, green: 1, blue: 1, alpha: 1),
        ] as CFDictionary)
        guard CGImageDestinationFinalize(destination), let provider = CGDataProvider(data: data as CFData),
              let jpeg = CGImage(jpegDataProviderSource: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent)
        else { return bitmap }
        return jpeg
    }
}
