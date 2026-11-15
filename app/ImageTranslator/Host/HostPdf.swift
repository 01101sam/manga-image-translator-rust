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

    static func render(document url: URL, page index: Int, longEdge: Int, to output: URL) throws {
        try autoreleasepool {
            guard let document = CGPDFDocument(url as CFURL) else { throw HostCallError("打不开这个 PDF") }
            if document.isEncrypted, !document.isUnlocked, !document.unlockWithPassword("") {
                throw HostCallError("这个 PDF 有密码, 暂不支持")
            }
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