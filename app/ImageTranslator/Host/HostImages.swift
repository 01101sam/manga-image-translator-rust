import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// 图片进沙盒时在宿主里做的加工。所有入口 (拖放、选择、粘贴、分享收件箱、打开方式、链接) 都经过 `ManagedFiles.adopt`, 所以在那里做一次就够。
enum ImagePrep {
    /// Daemon 的 image crate (默认特性) 按文件内容能解码的格式, ImageIO 报出的类型属于这里就原样放行。
    /// HEIC、HEIF、AVIF (crate 里只有 AVIF 编码)、JPEG 2000、PSD、TGA 等不在其中, 会被转成 JPEG (实测: HEIC 与 AVIF 都是 400)。
    static let daemonReadable: [UTType] = [.png, .jpeg, .gif, .webP, .tiff, .bmp, .ico]

    /// 转出来的 JPEG 的质量。
    static let jpegQuality = 0.92

    /// 转码时长边的上限, 免得异常的文件占掉过多内存; 不放大。
    static let maxConvertedEdge = 16_384

    static func needsConversion(_ typeIdentifier: String) -> Bool {
        guard let type = UTType(typeIdentifier) else { return true }
        return !daemonReadable.contains { type.conforms(to: $0) }
    }

    /// 文件按内容是 Daemon 读不了、ImageIO 却认得的图片时, 转成同目录下的 JPEG 并删掉原文件, 返回新文件; 不需要转或转不了返回 nil。
    /// 转不了就保持原样: 上传时 Daemon 会如实拒绝, 列表里显示「格式不支持」, 比悄悄丢掉文件好。
    static func convertIfNeeded(at url: URL) -> URL? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
              CGImageSourceGetCount(source) > 0,
              let typeIdentifier = CGImageSourceGetType(source) as String?,
              needsConversion(typeIdentifier)
        else { return nil }
        let directory = url.deletingLastPathComponent()
        let stem = (url.lastPathComponent as NSString).deletingPathExtension
        let working = directory.appendingPathComponent(".converting-\(UUID().uuidString).jpg")
        guard writeJPEG(from: source, to: working) else {
            try? FileManager.default.removeItem(at: working)
            return nil
        }
        let converted = directory.appendingPathComponent("\(stem.isEmpty ? "image" : stem).jpg")
        do {
            try FileManager.default.removeItem(at: url)
            if FileManager.default.fileExists(atPath: converted.path) { try FileManager.default.removeItem(at: converted) }
            try FileManager.default.moveItem(at: working, to: converted)
            return converted
        } catch {
            try? FileManager.default.removeItem(at: working)
            return nil
        }
    }

    /// 列表缩略图的长边 (像素)。列表和网格的格子最大约 200 点, 256 像素在 1x 屏上够用, 2x 屏上略糊但只是占位, 上传后换成 Daemon 的预览。
    static let thumbnailEdge = 256

    /// 缩略图 data URI 的字符数上限。它随文件引用进 JS 堆 (上限 256 MB), 一次导入几千张时每张都占这么多, 所以压到 16 KB 以内; 排版很密的漫画页压到这个大小也还能认出是哪一页。
    static let thumbnailBudget = 16_000

    /// 对放进沙盒的图片读到的东西: 方向修正后的像素宽高、列表用的小缩略图和相似图片的指纹。
    struct Inspection: Equatable {
        var width: Int
        var height: Int
        /// data:image/jpeg;base64,... 读不出来 (文件损坏、ImageIO 解不开) 时为 nil: 条目照常显示名字和状态, 只是没有缩略图。
        var thumb: String?
        /// 256 位 dHash, 64 个十六进制字符 (见 ImageFingerprint)。解不出来为 nil: 这张图不参与去重, 照常导入。
        var hash: String?
    }

    /// 一次打开文件, 取尺寸、缩略图和指纹。ImageIO 的缩略图解码对 JPEG 用 DCT 缩放、对 PNG 逐行缩小, 不会为了 256 像素把整张图解进内存;
    /// 缩略图和指纹用同一次解码出来的 256 像素的图 (指纹是格子平均, 不在乎这一步用什么滤波), 不为去重再解码一遍。
    static func inspect(at url: URL) -> Inspection? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
              CGImageSourceGetCount(source) > 0,
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int, width > 0, height > 0
        else { return nil }
        // EXIF 方向 5 到 8 是转 90 度: 显示出来的宽高互换。
        let turned = (5...8).contains(properties[kCGImagePropertyOrientation] as? Int ?? 1)
        let reduced = reducedImage(from: source, edge: thumbnailEdge)
        return Inspection(
            width: turned ? height : width, height: turned ? width : height,
            thumb: reduced.flatMap { thumbnailURI(first: $0, from: source) }, hash: reduced.flatMap(ImageFingerprint.hash(of:))
        )
    }

    static func reducedImage(from source: CGImageSource, edge: Int) -> CGImage? {
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: edge,
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }

    /// 先试 256 像素 (first 是已经解好的那一幅), 压不进预算就依次缩到 192、128; 每个尺寸试几档质量。
    static func thumbnailURI(first: CGImage, from source: CGImageSource) -> String? {
        var image = first
        for edge in [thumbnailEdge, 192, 128] {
            if edge != thumbnailEdge {
                guard let smaller = reducedImage(from: source, edge: edge) else { return nil }
                image = smaller
            }
            for quality in [0.7, 0.5, 0.35] {
                guard let data = jpegData(image, quality: quality) else { continue }
                let uri = "data:image/jpeg;base64," + data.base64EncodedString()
                if uri.count <= thumbnailBudget { return uri }
            }
        }
        return nil
    }

    static func jpegData(_ image: CGImage, quality: Double) -> Data? {
        let data = NSMutableData()
        guard let target = CGImageDestinationCreateWithData(data, UTType.jpeg.identifier as CFString, 1, nil) else { return nil }
        CGImageDestinationAddImage(target, image, [kCGImageDestinationLossyCompressionQuality: quality] as CFDictionary)
        return CGImageDestinationFinalize(target) ? data as Data : nil
    }

    /// 解码一次、编码一次。Daemon 不读 EXIF 方向, 所以方向 (kCGImageSourceCreateThumbnailWithTransform) 烘进像素, 输出不带方向标记; 宽色域转 sRGB。
    private static func writeJPEG(from source: CGImageSource, to destination: URL) -> Bool {
        guard let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int, width > 0, height > 0
        else { return false }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: min(max(width, height), maxConvertedEdge),
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary),
              let target = CGImageDestinationCreateWithURL(destination as CFURL, UTType.jpeg.identifier as CFString, 1, nil)
        else { return false }
        CGImageDestinationAddImage(target, image, [
            kCGImageDestinationLossyCompressionQuality: jpegQuality,
            kCGImageDestinationOptimizeColorForSharing: true,
        ] as CFDictionary)
        return CGImageDestinationFinalize(target)
    }
}