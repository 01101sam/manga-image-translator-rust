import ImageIO
import UIKit
import UniformTypeIdentifiers
import XCTest
@testable import ImageTranslator

/// Daemon 读不了的图片格式在导入时转成 JPEG: 内容决定要不要转, 方向烘进像素, 名字的主干保留。
final class HostImagesTests: XCTestCase {
    private var directory: URL!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("host-images-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: directory)
    }

    private func makeFiles() -> ManagedFiles {
        ManagedFiles(root: directory.appendingPathComponent("tmp"), pdfRoot: directory.appendingPathComponent("pdfs"))
    }

    /// 左半红、右半蓝的图, 方向和格式由参数决定; 编码器不支持这个格式时跳过测试。
    private func makeImage(_ name: String, type: UTType, width: Int = 300, height: Int = 200, orientation: Int? = nil) throws -> URL {
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = true
        let image = UIGraphicsImageRenderer(size: CGSize(width: width, height: height), format: format).image { context in
            UIColor.red.setFill()
            context.fill(CGRect(x: 0, y: 0, width: width / 2, height: height))
            UIColor.blue.setFill()
            context.fill(CGRect(x: width / 2, y: 0, width: width - width / 2, height: height))
        }
        let url = directory.appendingPathComponent(name)
        guard let destination = CGImageDestinationCreateWithURL(url as CFURL, type.identifier as CFString, 1, nil), let cgImage = image.cgImage else {
            throw XCTSkip("this device cannot encode \(type.identifier)")
        }
        var properties: [CFString: Any] = [kCGImageDestinationLossyCompressionQuality: 0.9]
        if let orientation { properties[kCGImagePropertyOrientation] = orientation }
        CGImageDestinationAddImage(destination, cgImage, properties as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { throw XCTSkip("this device cannot encode \(type.identifier)") }
        return url
    }

    private struct Decoded {
        var width: Int
        var height: Int
        var orientation: Int?
        var type: String?
        var red: (Int, Int) -> Bool
        var blue: (Int, Int) -> Bool
    }

    private func decode(_ url: URL) throws -> Decoded {
        let source = try XCTUnwrap(CGImageSourceCreateWithURL(url as CFURL, nil))
        let properties = try XCTUnwrap(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
        let image = try XCTUnwrap(CGImageSourceCreateImageAtIndex(source, 0, nil))
        func rgb(_ x: Int, _ y: Int) -> (Int, Int, Int) {
            var pixel = [UInt8](repeating: 0, count: 4)
            let context = CGContext(data: &pixel, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
            context.draw(image, in: CGRect(x: -x, y: -(image.height - 1 - y), width: image.width, height: image.height))
            return (Int(pixel[0]), Int(pixel[1]), Int(pixel[2]))
        }
        return Decoded(
            width: image.width, height: image.height, orientation: properties[kCGImagePropertyOrientation] as? Int, type: CGImageSourceGetType(source) as String?,
            red: { let (r, g, b) = rgb($0, $1); return r > 180 && g < 90 && b < 90 },
            blue: { let (r, g, b) = rgb($0, $1); return b > 180 && r < 90 && g < 90 }
        )
    }

    func testAHEICPhotoBecomesAJPEGWithTheSameNameStem() throws {
        let source = try makeImage("IMG_0001.HEIC", type: .heic)
        let files = makeFiles()
        let ref = try files.adopt(source, move: true)
        XCTAssertEqual(ref.name, "IMG_0001.jpg")
        XCTAssertEqual(ref.type, "image/jpeg")
        XCTAssertFalse(FileManager.default.fileExists(atPath: source.path))
        let decoded = try decode(try XCTUnwrap(files.url(for: ref.id)))
        XCTAssertEqual(decoded.type, UTType.jpeg.identifier)
        XCTAssertEqual([decoded.width, decoded.height], [300, 200], "the conversion must not resize")
        XCTAssertTrue(decoded.red(40, 100))
        XCTAssertTrue(decoded.blue(260, 100))
    }

    func testTheEXIFOrientationIsBakedIntoThePixelsBecauseTheDaemonIgnoresIt() throws {
        // 方向 6: 显示时顺时针转 90 度, 所以左半的红色到了上半。
        let source = try makeImage("portrait.heic", type: .heic, orientation: 6)
        let files = makeFiles()
        let ref = try files.adopt(source, move: true)
        let decoded = try decode(try XCTUnwrap(files.url(for: ref.id)))
        XCTAssertEqual([decoded.width, decoded.height], [200, 300])
        XCTAssertTrue(decoded.orientation == nil || decoded.orientation == 1, "the output must not carry an orientation tag")
        XCTAssertTrue(decoded.red(100, 40), "the red half must be at the top after rotating")
        XCTAssertTrue(decoded.blue(100, 260))
    }

    func testTheContentDecidesNotTheExtension() throws {
        let source = try makeImage("scan.png", type: .heic)
        let files = makeFiles()
        let ref = try files.adopt(source, move: true)
        XCTAssertEqual(ref.name, "scan.jpg")
        XCTAssertEqual(ref.type, "image/jpeg")
    }

    func testFormatsTheDaemonReadsAreLeftByteForByte() throws {
        let files = makeFiles()
        for (name, type) in [("a.png", UTType.png), ("b.jpg", .jpeg), ("c.tiff", .tiff), ("d.gif", .gif), ("e.bmp", .bmp)] {
            let source = try makeImage(name, type: type)
            let bytes = try Data(contentsOf: source)
            let ref = try files.adopt(source, move: false)
            XCTAssertEqual(ref.name, name)
            XCTAssertEqual(try Data(contentsOf: try XCTUnwrap(files.url(for: ref.id))), bytes, name)
        }
    }

    func testAnAVIFIsConvertedToo() throws {
        let source = try makeImage("photo.avif", type: UTType("public.avif") ?? .heic)
        let files = makeFiles()
        let ref = try files.adopt(source, move: true)
        XCTAssertEqual(ref.name, "photo.jpg")
    }

    func testAFileImageIOCannotDecodeIsKeptAsIsSoTheDaemonCanSayWhy() throws {
        let junk = directory.appendingPathComponent("broken.heic")
        try Data(repeating: 9, count: 64).write(to: junk)
        let ref = try makeFiles().adopt(junk, move: true)
        XCTAssertEqual(ref.name, "broken.heic")
        XCTAssertEqual(ref.size, 64)
    }

    func testConversionDecisionFollowsWhatTheDaemonDecodes() {
        for type in [UTType.png, .jpeg, .gif, .webP, .tiff, .bmp] { XCTAssertFalse(ImagePrep.needsConversion(type.identifier), type.identifier) }
        for type in [UTType.heic, .heif, UTType("public.avif"), UTType("public.jpeg-2000"), UTType("com.adobe.photoshop-image"), UTType("com.truevision.tga-image")] {
            XCTAssertTrue(ImagePrep.needsConversion(try! XCTUnwrap(type).identifier), type?.identifier ?? "")
        }
    }

    /// 用户的真机复现: 分享 13 张, 其中两张是 HEIC。十三张都要进来, 两张 HEIC 转成 JPEG、名字还在。
    func testABatchWithTwoHEICPhotosImportsAllThirteenAndConvertsTheTwo() async throws {
        let drop = directory.appendingPathComponent("drop")
        try FileManager.default.createDirectory(at: drop, withIntermediateDirectories: true)
        var sources: [URL] = []
        for index in 0..<13 {
            let heic = index == 3 || index == 9
            let made = try makeImage(String(format: "IMG_%04d.%@", index, heic ? "HEIC" : "PNG"), type: heic ? .heic : .png, width: 90, height: 60)
            let target = drop.appendingPathComponent(made.lastPathComponent)
            try FileManager.default.moveItem(at: made, to: target)
            sources.append(target)
        }
        // 相册和分享面板给的 provider 带着用户看到的名字; 进程内用文件造的 provider 没有, 临时副本叫 "HEIF图像" 这类通用名 (实测), 所以这里补上。
        let providers = sources.compactMap { source -> NSItemProvider? in
            let provider = NSItemProvider(contentsOf: source)
            provider?.suggestedName = source.deletingPathExtension().lastPathComponent
            return provider
        }
        let result = await FileIngestor(files: makeFiles()).ingest(providers: providers, allowing: [.image, .pdf])
        XCTAssertEqual(result.images.count, 13)
        XCTAssertEqual(result.ignored + result.failed, 0)
        XCTAssertEqual(result.images.map(\.name).filter { $0.hasSuffix(".jpg") }, ["IMG_0003.jpg", "IMG_0009.jpg"])
        XCTAssertEqual(Set(result.images.map(\.type)), ["image/png", "image/jpeg"])
    }

    func testAProviderThatFailsToDeliverItsFileCountsAsAReadFailureNotAsNotAnImage() async throws {
        let provider = NSItemProvider()
        provider.registerFileRepresentation(forTypeIdentifier: UTType.jpeg.identifier, fileOptions: [], visibility: .all) { completion in
            completion(nil, false, NSError(domain: NSURLErrorDomain, code: NSURLErrorNotConnectedToInternet))
            return nil
        }
        let result = await FileIngestor(files: makeFiles()).ingest(providers: [provider], allowing: [.image])
        XCTAssertEqual(result.failed, 1)
        XCTAssertEqual(result.ignored, 0)
        let events = ImportDelivery(result).events()
        XCTAssertEqual(events.count, 1)
        XCTAssertTrue(((events[0].payload as? [String: String])?["text"] ?? "").contains("没能读进来"))
    }

    func testTheSharedNameWinsOverTheProvidersGenericTemporaryName() {
        let temporary = URL(fileURLWithPath: "/tmp/image.heic")
        XCTAssertEqual(FileNaming.name(for: temporary, suggested: "IMG_4321", type: .heic), "IMG_4321.heic")
        XCTAssertEqual(FileNaming.name(for: temporary, suggested: "IMG_4321.HEIC", type: .heic), "IMG_4321.HEIC")
        XCTAssertEqual(FileNaming.name(for: temporary, suggested: nil, type: .heic), "image.heic")
        XCTAssertEqual(FileNaming.name(for: URL(fileURLWithPath: "/tmp/blob"), suggested: "../a/b:c", type: .png), "b_c.png")
    }

    func testTheShareExtensionKeepsTheNameThePhotosAppShows() async throws {
        let inbox = directory.appendingPathComponent("inbox", isDirectory: true)
        try FileManager.default.createDirectory(at: inbox, withIntermediateDirectories: true)
        let source = try makeImage("generic.jpeg", type: .jpeg, width: 30, height: 20)
        let provider = try XCTUnwrap(DebugProvider.make(source))
        provider.suggestedName = "IMG_7788"
        let summary = await ShareDepositor(inbox: inbox).deposit([provider])
        XCTAssertEqual(summary.added, 1)
        let payload = try XCTUnwrap(ShareInbox.entries(in: inbox).first.flatMap(ShareInbox.payload(of:)))
        XCTAssertEqual(payload.lastPathComponent, "IMG_7788.jpeg")
    }
}