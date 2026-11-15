import ImageIO
import UIKit
import XCTest
@testable import ImageTranslator

final class HostFilesTests: XCTestCase {
    private var directory: URL!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("host-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: directory)
    }

    private func makeFiles() -> ManagedFiles {
        ManagedFiles(root: directory.appendingPathComponent("tmp"), pdfRoot: directory.appendingPathComponent("pdfs"))
    }

    private func write(_ name: String, bytes: Int = 16, in folder: URL? = nil) throws -> URL {
        let parent = folder ?? directory!
        try FileManager.default.createDirectory(at: parent, withIntermediateDirectories: true)
        let url = parent.appendingPathComponent(name)
        try Data(repeating: 7, count: bytes).write(to: url)
        return url
    }

    private func makePDF(pages: Int) throws -> URL {
        let url = directory.appendingPathComponent("doc-\(UUID().uuidString).pdf")
        let renderer = UIGraphicsPDFRenderer(bounds: CGRect(x: 0, y: 0, width: 612, height: 792))
        try renderer.writePDF(to: url) { context in
            for page in 0..<pages {
                context.beginPage()
                ("page \(page + 1)" as NSString).draw(at: CGPoint(x: 72, y: 72), withAttributes: [.font: UIFont.systemFont(ofSize: 48)])
            }
        }
        return url
    }

    func testAdoptMovesTheFileAndReleaseDeletesIt() throws {
        let files = makeFiles()
        let source = try write("page01.png", bytes: 1234)
        let ref = try files.adopt(source, move: true)
        XCTAssertEqual(ref.name, "page01.png")
        XCTAssertEqual(ref.size, 1234)
        XCTAssertEqual(ref.type, "image/png")
        XCTAssertFalse(FileManager.default.fileExists(atPath: source.path))
        let stored = try XCTUnwrap(files.url(for: ref.id))
        XCTAssertTrue(FileManager.default.fileExists(atPath: stored.path))
        files.release([ref.id])
        XCTAssertNil(files.url(for: ref.id))
        XCTAssertFalse(FileManager.default.fileExists(atPath: stored.path))
    }

    func testAdoptCopiesWhenAsked() throws {
        let files = makeFiles()
        let source = try write("keep.jpg")
        _ = try files.adopt(source, move: false)
        XCTAssertTrue(FileManager.default.fileExists(atPath: source.path))
    }

    func testReleaseIgnoresUnknownIDs() {
        let files = makeFiles()
        files.release(["../../etc", "nope"])
    }

    func testPDFImportKeepsNameAndPageCountAndResolvesByUUIDOnly() throws {
        let files = makeFiles()
        let source = try makePDF(pages: 3)
        let ref = try files.importPDF(from: source, name: "book.pdf", move: false)
        XCTAssertEqual(ref.pageCount, 3)
        XCTAssertEqual(ref.type, "application/pdf")
        XCTAssertNotNil(files.pdfURL(for: ref.id))
        XCTAssertEqual(files.pdfDisplayName(for: ref.id), "book.pdf")
        XCTAssertNil(files.pdfURL(for: "../pdfs/\(ref.id)"))
        files.releasePDF(ref.id)
        XCTAssertNil(files.pdfURL(for: ref.id))
    }

    func testCleanOnLaunchDropsTransientFilesAndStalePDFsOnly() throws {
        let files = makeFiles()
        let image = try files.adopt(try write("a.png"), move: true)
        let stale = try files.importPDF(from: try makePDF(pages: 1), move: false)
        let fresh = try files.importPDF(from: try makePDF(pages: 1), move: false)
        let staleURL = try XCTUnwrap(files.pdfURL(for: stale.id))
        try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSinceNow: -31 * 24 * 3600)], ofItemAtPath: staleURL.path)
        files.cleanOnLaunch()
        XCTAssertNil(files.url(for: image.id))
        XCTAssertNil(files.pdfURL(for: stale.id))
        XCTAssertNotNil(files.pdfURL(for: fresh.id))
    }

    func testSafeFileNameStripsPathsAndControlCharacters() {
        XCTAssertEqual(ManagedFiles.safeFileName("../../etc/passwd", fallback: "x"), "passwd")
        XCTAssertEqual(ManagedFiles.safeFileName("a\nb.png", fallback: "x"), "ab.png")
        XCTAssertEqual(ManagedFiles.safeFileName("..", fallback: "x"), "x")
        XCTAssertEqual(ManagedFiles.safeFileName("", fallback: "x"), "x")
    }

    func testFolderIngestRecursesInNaturalOrderAndSkipsOtherFiles() async throws {
        let files = makeFiles()
        let folder = directory.appendingPathComponent("chapter")
        _ = try write("page10.png", in: folder)
        _ = try write("page2.png", in: folder)
        _ = try write("notes.txt", in: folder)
        _ = try write("page1.jpg", in: folder.appendingPathComponent("extra"))
        let result = await FileIngestor(files: files).ingest(urls: [folder], allowing: [.image])
        // 按相对路径排: extra/ 在 page* 之前, page2 在 page10 之前。
        XCTAssertEqual(result.images.map(\.name), ["page1.jpg", "page2.png", "page10.png"])
        XCTAssertTrue(result.pdfs.isEmpty)
        XCTAssertTrue(FileManager.default.fileExists(atPath: folder.appendingPathComponent("page2.png").path))
    }

    func testProviderIngestHandlesManyFilesWithOneRepresentationEach() async throws {
        let files = makeFiles()
        let sources = try (0..<12).map { try write("p\($0).png", bytes: 64, in: directory.appendingPathComponent("drop")) }
        let providers = sources.compactMap { NSItemProvider(contentsOf: $0) }
        XCTAssertEqual(providers.count, 12)
        let result = await FileIngestor(files: files).ingest(providers: providers, allowing: [.image, .pdf])
        XCTAssertEqual(result.images.count, 12)
        XCTAssertEqual(result.ignored, 0)
        XCTAssertEqual(Set(result.images.map(\.size)), [64])
    }

    func testProviderIngestIgnoresKindsTheCallerDidNotAllow() async throws {
        let files = makeFiles()
        let pdf = try makePDF(pages: 1)
        let provider = try XCTUnwrap(NSItemProvider(contentsOf: pdf))
        let result = await FileIngestor(files: files).ingest(providers: [provider], allowing: [.image])
        XCTAssertEqual(result.ignored, 1)
        XCTAssertTrue(result.pdfs.isEmpty)
    }

    func testLimiterNeverRunsMoreThanLimitAtOnce() async {
        let limiter = AsyncLimiter(limit: 3)
        let counter = Counter()
        await withTaskGroup(of: Void.self) { group in
            for _ in 0..<20 {
                group.addTask {
                    await limiter.run {
                        await counter.enter()
                        try? await Task.sleep(nanoseconds: 5_000_000)
                        await counter.leave()
                    }
                }
            }
        }
        let peak = await counter.peak
        XCTAssertLessThanOrEqual(peak, 3)
        XCTAssertGreaterThan(peak, 1)
    }

    func testRenderPageProducesAPNGWithTheRequestedLongEdge() throws {
        let pdf = try makePDF(pages: 2)
        let output = directory.appendingPathComponent("page.png")
        try PdfRenderer.render(document: pdf, page: 1, longEdge: 1000, to: output)
        let source = try XCTUnwrap(CGImageSourceCreateWithURL(output as CFURL, nil))
        let properties = try XCTUnwrap(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
        XCTAssertEqual(properties[kCGImagePropertyPixelHeight] as? Int, 1000)
        XCTAssertEqual(properties[kCGImagePropertyPixelWidth] as? Int, 773)
        XCTAssertThrowsError(try PdfRenderer.render(document: pdf, page: 2, longEdge: 1000, to: output))
    }

    /// 页面比画布小时 getDrawingTransform 只会居中不会放大: 渲染出的内容必须铺满画布, 否则送去翻译的图里页面只占中间一小块。
    func testRenderPageFillsTheCanvasInsteadOfStayingAtPointSize() throws {
        let bounds = CGRect(x: 0, y: 0, width: 612, height: 792)
        let pdf = directory.appendingPathComponent("full-bleed.pdf")
        try UIGraphicsPDFRenderer(bounds: bounds).writePDF(to: pdf) { context in
            context.beginPage()
            UIColor.black.setFill()
            context.fill(bounds)
        }
        let output = directory.appendingPathComponent("full-bleed.png")
        try PdfRenderer.render(document: pdf, page: 0, longEdge: 2000, to: output)
        let image = try XCTUnwrap(CGImageSourceCreateWithURL(output as CFURL, nil).flatMap { CGImageSourceCreateImageAtIndex($0, 0, nil) })
        let corners = [(2, 2), (image.width - 3, 2), (2, image.height - 3), (image.width - 3, image.height - 3)]
        for (x, y) in corners {
            XCTAssertLessThan(try Self.brightness(of: image, x: x, y: y), 16, "pixel (\(x), \(y)) of \(image.width)x\(image.height) should be page content, not margin")
        }
    }

    /// 取一个像素的亮度 (0 到 255): 把整张图画进一个已知格式的位图再读, 不依赖 PNG 自己的像素布局。
    private static func brightness(of image: CGImage, x: Int, y: Int) throws -> Int {
        var pixel = [UInt8](repeating: 0, count: 4)
        let drawn = pixel.withUnsafeMutableBytes { buffer -> Bool in
            guard let context = CGContext(
                data: buffer.baseAddress, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            ) else { return false }
            context.draw(image, in: CGRect(x: -x, y: -(image.height - 1 - y), width: image.width, height: image.height))
            return true
        }
        guard drawn else { throw XCTSkip("no bitmap context") }
        return (Int(pixel[0]) + Int(pixel[1]) + Int(pixel[2])) / 3
    }

    func testDisplayNamePrefersTheSuggestedNameAndKeepsOneExtension() {
        let temp = URL(fileURLWithPath: "/tmp/PNG图像.png")
        XCTAssertEqual(FileIngestor.name(for: temp, suggested: "page01", type: .png), "page01.png")
        XCTAssertEqual(FileIngestor.name(for: temp, suggested: "page01.PNG", type: .png), "page01.PNG")
        XCTAssertEqual(FileIngestor.name(for: temp, suggested: nil, type: .png), "PNG图像.png")
        XCTAssertEqual(FileIngestor.name(for: URL(fileURLWithPath: "/tmp/blob"), suggested: "scan", type: .jpeg), "scan.jpeg")
    }

    func testLongEdgeIsClamped() {
        XCTAssertEqual(PdfRenderer.clampedLongEdge(nil), 3072)
        XCTAssertEqual(PdfRenderer.clampedLongEdge(10), 256)
        XCTAssertEqual(PdfRenderer.clampedLongEdge(100_000), 6144)
    }
}

private actor Counter {
    private var current = 0
    private(set) var peak = 0
    func enter() { current += 1; peak = max(peak, current) }
    func leave() { current -= 1 }
}
