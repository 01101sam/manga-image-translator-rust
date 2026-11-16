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
