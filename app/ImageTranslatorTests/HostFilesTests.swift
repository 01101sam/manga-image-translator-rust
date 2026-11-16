import ImageIO
import UIKit
import UniformTypeIdentifiers
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

    private func stamp(_ url: URL, _ seconds: TimeInterval) throws {
        try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: seconds)], ofItemAtPath: url.path)
    }

    func testAdoptReportsTheSourceFilesModificationTimeInMilliseconds() throws {
        let files = makeFiles()
        let moved = try write("moved.png")
        try stamp(moved, 1_700_000_000)
        let ref = try files.adopt(moved, move: true)
        XCTAssertEqual(ref.modifiedAt, 1_700_000_000_000)
        XCTAssertEqual(ref.json["modifiedAt"] as? Double, 1_700_000_000_000)
        let copied = try write("copied.png")
        try stamp(copied, 1_600_000_123.456)
        XCTAssertEqual(try files.adopt(copied, move: false).modifiedAt, 1_600_000_123_456)
    }

    func testAdoptWithoutADateLeavesItOutOfTheReference() throws {
        let files = makeFiles()
        let source = try write("rendered.png")
        try stamp(source, 1_700_000_000)
        let ref = try files.adopt(source, move: true, dating: .none)
        XCTAssertNil(ref.modifiedAt)
        XCTAssertNil(ref.json["modifiedAt"])
        XCTAssertNil(ManagedFiles.modificationTime(of: directory.appendingPathComponent("missing.png")))
    }

    func testFolderIngestKeepsEachFilesOwnModificationTime() async throws {
        let files = makeFiles()
        let folder = directory.appendingPathComponent("dated")
        try stamp(try write("a.png", in: folder), 1_700_000_300)
        try stamp(try write("b.png", in: folder), 1_700_000_100)
        let result = await FileIngestor(files: files).ingest(urls: [folder], allowing: [.image])
        XCTAssertEqual(result.images.map(\.name), ["a.png", "b.png"])
        XCTAssertEqual(result.images.map(\.modifiedAt), [1_700_000_300_000, 1_700_000_100_000])
    }

    /// 实测 (模拟器, 进程内 provider): provider 的文件表示是临时副本, 修改时间是复制的那一刻 (1794805617491, 而源文件是 1700000000000),
    /// 所以拖放和照片选择器的文件在这里没有修改时间, 而不是带一个看着像真的导入时间。
    func testProviderCopiesCarryNoModificationTimeBecauseItIsOnlyTheCopyTime() async throws {
        let files = makeFiles()
        let source = try write("provided.png", bytes: 64, in: directory.appendingPathComponent("drop"))
        try stamp(source, 1_700_000_000)
        let provider = try XCTUnwrap(NSItemProvider(contentsOf: source))
        let result = await FileIngestor(files: files).ingest(providers: [provider], allowing: [.image])
        let ref = try XCTUnwrap(result.images.first)
        XCTAssertEqual(ref.size, 64)
        XCTAssertNil(ref.modifiedAt)
    }

    func testProviderCopyDatingKeepsOldTimesAndDropsJustCopiedOnes() throws {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let old = try write("old.png")
        try stamp(old, 1_700_000_000)
        let fresh = try write("fresh.png")
        try stamp(fresh, 1_800_000_000 - 30)
        let borderline = try write("edge.png")
        try stamp(borderline, 1_800_000_000 - 121)
        XCTAssertEqual(ManagedFiles.modifiedAt(of: old, dating: .providerCopy, now: now), 1_700_000_000_000)
        XCTAssertNil(ManagedFiles.modifiedAt(of: fresh, dating: .providerCopy, now: now))
        XCTAssertEqual(ManagedFiles.modifiedAt(of: borderline, dating: .providerCopy, now: now), 1_799_999_879_000)
        XCTAssertEqual(ManagedFiles.modifiedAt(of: fresh, dating: .original, now: now), 1_799_999_970_000)
        XCTAssertNil(ManagedFiles.modifiedAt(of: old, dating: .none, now: now))
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

    func testCleanOnLaunchDropsScratchAndOldFilesButKeepsRecentImagesAndPDFs() throws {
        let files = makeFiles()
        let image = try files.adopt(try write("a.png"), move: true)
        let oldImage = try files.adopt(try write("old.png"), move: true)
        let oldDirectory = try XCTUnwrap(files.url(for: oldImage.id)).deletingLastPathComponent()
        try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSinceNow: -15 * 24 * 3600)], ofItemAtPath: oldDirectory.path)
        let scratch = files.scratchFile(named: "upload.body")
        try Data("x".utf8).write(to: scratch)
        let stale = try files.importPDF(from: try makePDF(pages: 1), move: false)
        let fresh = try files.importPDF(from: try makePDF(pages: 1), move: false)
        let staleURL = try XCTUnwrap(files.pdfURL(for: stale.id))
        try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSinceNow: -31 * 24 * 3600)], ofItemAtPath: staleURL.path)
        files.cleanOnLaunch()
        XCTAssertNotNil(files.url(for: image.id), "an image that may still be in the saved list must survive a launch")
        XCTAssertNil(files.url(for: oldImage.id), "an image nobody has claimed for two weeks is an orphan")
        XCTAssertFalse(FileManager.default.fileExists(atPath: scratch.path))
        XCTAssertNil(files.pdfURL(for: stale.id))
        XCTAssertNotNil(files.pdfURL(for: fresh.id))
    }

    /// 用户的真机复现之一: 待上传的图片只在 JS 内存里, App 在上传前被杀就没了。现在存档里有 fileId, 重启后宿主凭它找回文件和缩略图。
    func testAnImageIsFoundByItsIDAfterARestartWithItsNameSizeAndThumbnail() throws {
        let source = directory.appendingPathComponent("page.png")
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        try UIGraphicsImageRenderer(size: CGSize(width: 120, height: 80), format: format).pngData { context in
            UIColor.red.setFill()
            context.fill(CGRect(x: 0, y: 0, width: 120, height: 80))
        }.write(to: source)
        let before = makeFiles()
        let ref = try before.adopt(source, move: true)
        XCTAssertNotNil(ref.thumb)
        let after = makeFiles()
        after.cleanOnLaunch()
        let found = after.resolve([ref.id, UUID().uuidString, "../etc", ""])
        XCTAssertEqual(found.map(\.id), [ref.id])
        XCTAssertEqual(found.first?.name, "page.png")
        XCTAssertEqual(found.first?.size, ref.size)
        XCTAssertEqual(found.first?.type, "image/png")
        XCTAssertEqual(found.first?.thumb, ref.thumb)
        XCTAssertEqual(after.url(for: ref.id)?.lastPathComponent, "page.png")
    }

    func testResolvingRefreshesTheAgeSoAClaimedImageSurvivesTheNextSweepAndTheOrphanDoesNot() throws {
        let files = makeFiles()
        let claimed = try files.adopt(try write("claimed.png"), move: true)
        let orphan = try files.adopt(try write("orphan.png"), move: true)
        for ref in [claimed, orphan] {
            let folder = try XCTUnwrap(files.url(for: ref.id)).deletingLastPathComponent()
            try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSinceNow: -13 * 24 * 3600)], ofItemAtPath: folder.path)
        }
        XCTAssertEqual(files.resolve([claimed.id]).count, 1)
        files.cleanOnLaunch(now: Date(timeIntervalSinceNow: 3 * 24 * 3600))
        XCTAssertNotNil(files.url(for: claimed.id))
        XCTAssertNil(files.url(for: orphan.id))
    }

    func testReleaseAfterARestartStillDeletesTheFile() throws {
        let before = makeFiles()
        let ref = try before.adopt(try write("gone.png"), move: true)
        let stored = try XCTUnwrap(before.url(for: ref.id))
        let after = makeFiles()
        after.release([ref.id])
        XCTAssertFalse(FileManager.default.fileExists(atPath: stored.path))
        XCTAssertNil(after.url(for: ref.id))
    }

    func testSharedFileKeepsTheCleanNameInItsOwnDirectory() {
        let files = makeFiles()
        let first = files.sharedFile(named: "vol01-translated.pdf")
        let second = files.sharedFile(named: "vol01-translated.pdf")
        XCTAssertEqual(first.lastPathComponent, "vol01-translated.pdf")
        XCTAssertNotEqual(first.deletingLastPathComponent(), second.deletingLastPathComponent())
        XCTAssertTrue(FileManager.default.fileExists(atPath: first.deletingLastPathComponent().path))
        XCTAssertEqual(files.sharedFile(named: "../../etc/passwd").lastPathComponent, "passwd")
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

    /// 进程内 provider 同时读多个大文件时, 读过的文件页会一直算在 RSS 里直到这一批结束 (30 张 37 MB 的 PNG 涨到 1.3 GB, 模拟器里测的)。
    /// 一次只读一个, RSS 只比起点多出一个文件的大小。footprint 不受影响, 但 RSS 涨到 GB 会让监控和排查都误判。
    func testProviderIngestOfBigFilesKeepsResidentMemoryNearOneFile() async throws {
        let files = makeFiles()
        let megabytes = 24
        let sources = try (0..<10).map { try write("big\($0).png", bytes: megabytes * 1_048_576, in: directory.appendingPathComponent("drop")) }
        let providers = sources.compactMap { NSItemProvider(contentsOf: $0) }
        XCTAssertEqual(providers.count, 10)
        let before = HostMetrics.residentMB()
        let (result, peak) = await HostMetrics.measure { await FileIngestor(files: files).ingest(providers: providers, allowing: [.image]) }
        XCTAssertEqual(result.images.count, 10)
        XCTAssertLessThan(peak.rss - before, megabytes * 4, "peak RSS grew by \(peak.rss - before) MB while reading \(10 * megabytes) MB of files")
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

    // MARK: - PDF 导出

    /// n 页纯色 PDF: 第 i 页的颜色不同, 读回来能认出是哪一页的哪个来源。
    private func makeColoredPDF(colors: [UIColor], size: CGSize = CGSize(width: 300, height: 400)) throws -> URL {
        let url = directory.appendingPathComponent("colored-\(UUID().uuidString).pdf")
        let renderer = UIGraphicsPDFRenderer(bounds: CGRect(origin: .zero, size: size))
        try renderer.writePDF(to: url) { context in
            for color in colors {
                context.beginPage()
                color.setFill()
                context.fill(CGRect(origin: .zero, size: size))
            }
        }
        return url
    }

    private func makeJPEG(color: UIColor, size: CGSize) throws -> URL {
        let url = directory.appendingPathComponent("artifact-\(UUID().uuidString).jpg")
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        let data = UIGraphicsImageRenderer(size: size, format: format).jpegData(withCompressionQuality: 0.9) { context in
            color.setFill()
            context.fill(CGRect(origin: .zero, size: size))
        }
        try data.write(to: url)
        return url
    }

    /// 把导出的某一页渲染成小图, 取中心像素的亮度和红色分量, 用来认出这一页用的是哪个来源。
    private func centerColor(of pdf: URL, page: Int) throws -> (red: Int, green: Int, blue: Int) {
        let output = directory.appendingPathComponent("probe-\(UUID().uuidString).png")
        try PdfRenderer.render(document: pdf, page: page, longEdge: 64, to: output)
        let image = try XCTUnwrap(CGImageSourceCreateWithURL(output as CFURL, nil).flatMap { CGImageSourceCreateImageAtIndex($0, 0, nil) })
        var pixel = [UInt8](repeating: 0, count: 4)
        pixel.withUnsafeMutableBytes { buffer in
            let context = CGContext(
                data: buffer.baseAddress, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            )
            context?.draw(image, in: CGRect(x: -image.width / 2, y: -image.height / 2, width: image.width, height: image.height))
        }
        return (Int(pixel[0]), Int(pixel[1]), Int(pixel[2]))
    }

    func testExportKeepsPageCountAndSizesAndSwapsOnlyTheTranslatedPages() async throws {
        let source = try makeColoredPDF(colors: [.red, .red, .red, .red])
        let translated = try makeJPEG(color: .blue, size: CGSize(width: 900, height: 1200))
        let output = directory.appendingPathComponent("out.pdf")
        let outcome = try await PdfExporter.compose(source: source, output: output) { index in index == 1 || index == 3 ? translated : nil }
        XCTAssertEqual(outcome, PdfExporter.Outcome(translated: 2, missing: 0))
        let document = try XCTUnwrap(CGPDFDocument(output as CFURL))
        XCTAssertEqual(document.numberOfPages, 4)
        for index in 1...4 {
            let size = PdfRenderer.displaySize(of: try XCTUnwrap(document.page(at: index)))
            XCTAssertEqual(size.width, 300, accuracy: 0.5)
            XCTAssertEqual(size.height, 400, accuracy: 0.5)
        }
        let originalPage = try centerColor(of: output, page: 0)
        XCTAssertGreaterThan(originalPage.red, 200)
        XCTAssertLessThan(originalPage.blue, 60)
        let translatedPage = try centerColor(of: output, page: 1)
        XCTAssertGreaterThan(translatedPage.blue, 200, "page 2 should show the translated image")
        XCTAssertLessThan(translatedPage.red, 60)
        XCTAssertGreaterThan(try centerColor(of: output, page: 2).red, 200)
        XCTAssertGreaterThan(try centerColor(of: output, page: 3).blue, 200)
    }

    func testExportFallsBackToTheOriginalPageWhenTheTranslationCannotBeFetchedOrRead() async throws {
        struct Gone: Error {}
        let source = try makeColoredPDF(colors: [.red, .red, .red])
        let notAnImage = try write("page.html", bytes: 64)
        let output = directory.appendingPathComponent("fallback.pdf")
        let outcome = try await PdfExporter.compose(source: source, output: output) { index in
            if index == 0 { throw Gone() }
            return index == 1 ? notAnImage : nil
        }
        XCTAssertEqual(outcome, PdfExporter.Outcome(translated: 0, missing: 2))
        XCTAssertEqual(CGPDFDocument(output as CFURL)?.numberOfPages, 3)
        for page in 0..<3 { XCTAssertGreaterThan(try centerColor(of: output, page: page).red, 200) }
    }

    func testExportReleasesEachArtifactRightAfterItsPage() async throws {
        let source = try makeColoredPDF(colors: [.red, .red, .red])
        let translated = try makeJPEG(color: .blue, size: CGSize(width: 90, height: 120))
        var events: [String] = []
        _ = try await PdfExporter.compose(
            source: source, output: directory.appendingPathComponent("order.pdf"),
            artifact: { index in events.append("get\(index)"); return translated },
            release: { _ in events.append("release") }
        )
        XCTAssertEqual(events, ["get0", "release", "get1", "release", "get2", "release"])
    }

    /// 每页一张不同的 2400x3200 噪声 JPEG (位图 30 MB, 文件约 7 MB): 24 页的位图加起来 700 MB 以上。
    /// 逐页处理时内存和页数无关; 全部留着的话这里会涨到 GB 级。
    func testExportOfManyBigPagesDoesNotGrowMemoryWithThePageCount() async throws {
        let pages = 24
        let source = try makeColoredPDF(colors: Array(repeating: .red, count: pages), size: CGSize(width: 612, height: 792))
        let (width, height) = (2400, 3200)
        func noisyJPEG(to url: URL) throws {
            var pixels = [UInt8](repeating: 0, count: width * height * 4)
            pixels.withUnsafeMutableBytes { arc4random_buf($0.baseAddress, $0.count) }
            let image = pixels.withUnsafeMutableBytes { buffer -> CGImage? in
                CGContext(
                    data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
                )?.makeImage()
            }
            let destination = try XCTUnwrap(CGImageDestinationCreateWithURL(url as CFURL, UTType.jpeg.identifier as CFString, 1, nil))
            CGImageDestinationAddImage(destination, try XCTUnwrap(image), [kCGImageDestinationLossyCompressionQuality: 0.9] as CFDictionary)
            XCTAssertTrue(CGImageDestinationFinalize(destination))
        }
        let before = HostMetrics.residentMB()
        let output = directory.appendingPathComponent("many.pdf")
        var failure: Error?
        let (outcome, peak) = await HostMetrics.measure {
            try? await PdfExporter.compose(
                source: source, output: output,
                artifact: { _ in
                    let url = self.directory.appendingPathComponent("noise-\(UUID().uuidString).jpg")
                    do { try noisyJPEG(to: url) } catch { failure = error }
                    return url
                },
                release: { try? FileManager.default.removeItem(at: $0) }
            )
        }
        XCTAssertNil(failure)
        XCTAssertEqual(outcome?.translated, pages)
        XCTAssertEqual(CGPDFDocument(output as CFURL)?.numberOfPages, pages)
        XCTAssertGreaterThan(ManagedFiles.size(of: output), pages * 100_000, "every page should carry its own image")
        // 实测增长约 100 MB (大头是测试自己生成的噪声图); 把 24 页降采样后的位图都留着会再多出约 180 MB。
        XCTAssertLessThan(peak.rss - before, 200, "peak RSS grew by \(peak.rss - before) MB for \(pages) pages")
    }

    // MARK: - 网页链接

    func testWebTypePrefersTheResponseTypeAndFallsBackToTheFileHeader() {
        XCTAssertEqual(FileIngestor.webType(mimeType: "image/jpeg", header: Data()), .jpeg)
        XCTAssertEqual(FileIngestor.webType(mimeType: "application/pdf", header: Data()), .pdf)
        XCTAssertEqual(FileIngestor.webType(mimeType: "application/octet-stream", header: Data("%PDF-1.4\n".utf8)), .pdf)
        XCTAssertEqual(FileIngestor.webType(mimeType: nil, header: Data([0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10])), .jpeg)
        XCTAssertEqual(FileIngestor.webType(mimeType: "binary/octet-stream", header: Data([0x89, 0x50, 0x4E, 0x47, 0x0D])), .png)
        XCTAssertEqual(FileIngestor.webType(mimeType: nil, header: Data("RIFF\0\0\0\0WEBPVP8 ".utf8)), .webP)
        XCTAssertNil(FileIngestor.webType(mimeType: "text/html", header: Data("<!doctype html>".utf8)))
        XCTAssertNil(FileIngestor.webType(mimeType: nil, header: Data()))
    }

    func testLinkNameIsTheLastPathSegmentWithoutPercentEncoding() {
        XCTAssertEqual(FileIngestor.linkName(URL(string: "https://example.com/a/page%2001.png?x=1")!), "page 01.png")
        XCTAssertNil(FileIngestor.linkName(URL(string: "https://example.com")!))
    }

    func testLinkFileNameKeepsAMatchingExtensionAndFixesTheRest() {
        let link = { (text: String) in URL(string: text)! }
        XCTAssertEqual(FileIngestor.linkFileName(link("https://x.test/a/doc.pdf"), type: .pdf), "doc.pdf")
        XCTAssertEqual(FileIngestor.linkFileName(link("https://x.test/a/photo.jpeg"), type: .jpeg), "photo.jpeg")
        XCTAssertEqual(FileIngestor.linkFileName(link("https://x.test/a/photo.jpg"), type: .jpeg), "photo.jpg")
        XCTAssertEqual(FileIngestor.linkFileName(link("https://x.test/a/page01"), type: .png), "page01.png")
        XCTAssertEqual(FileIngestor.linkFileName(link("https://x.test/a/page01.php"), type: .png), "page01.php.png")
        XCTAssertEqual(FileIngestor.linkFileName(link("https://x.test"), type: .pdf), "download.pdf")
    }

    func testDroppedWebLinksAreDownloadedAndRoutedByWhatTheyAre() async throws {
        let png = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0])
        LinkStub.responses = [
            "https://stub.test/img/page01": (200, "application/octet-stream", png),
            "https://stub.test/doc.pdf": (200, "application/pdf", Data("%PDF-1.4\n%".utf8)),
            "https://stub.test/index.html": (200, "text/html", Data("<html></html>".utf8)),
            "https://stub.test/missing.png": (404, "image/png", Data()),
        ]
        URLProtocol.registerClass(LinkStub.self)
        defer { URLProtocol.unregisterClass(LinkStub.self) }
        let files = makeFiles()
        let links = ["https://stub.test/img/page01", "https://stub.test/doc.pdf", "https://stub.test/index.html", "https://stub.test/missing.png", "ftp://stub.test/x.png"]
        let providers = links.map { NSItemProvider(object: NSURL(string: $0)!) }
        let result = await FileIngestor(files: files).ingest(providers: providers, allowing: [.image, .pdf])
        XCTAssertEqual(result.images.map(\.name), ["page01.png"])
        XCTAssertEqual(result.images.first?.type, "image/png")
        XCTAssertEqual(result.pdfs.map(\.name), ["doc.pdf"])
        XCTAssertEqual(result.ignored, 2, "an HTML page and a non-http link are not images")
        XCTAssertEqual(result.failed, 1, "a link that answers 404 could not be read, which is not the same as not being an image")
        let onlyImages = await FileIngestor(files: files).ingest(providers: [NSItemProvider(object: NSURL(string: "https://stub.test/doc.pdf")!)], allowing: [.image])
        XCTAssertEqual(onlyImages.ignored, 1)
        XCTAssertTrue(onlyImages.pdfs.isEmpty)
    }

    /// 手动测量用: 设 TEST_RUNNER_HOST_MANGA_PDF 指向一个很大的图片 PDF, 每 10 页换一张译文图, 其余页走原页 (drawPDFPage)。
    /// 这是用户真会碰到的路径: 绝大多数页是没翻译的原页, 原页里的大图被整页拷进新 PDF, 内存随页数线性增长 (见 app/README.md 的「内存」)。
    func testExportOfARealImagePDFReportsItsMemory() async throws {
        guard let path = ProcessInfo.processInfo.environment["HOST_MANGA_PDF"] else { throw XCTSkip("set TEST_RUNNER_HOST_MANGA_PDF to a large image PDF") }
        let source = URL(fileURLWithPath: path)
        let artifact = directory.appendingPathComponent("translated.jpg")
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        try UIGraphicsImageRenderer(size: CGSize(width: 1800, height: 2600), format: format)
            .jpegData(withCompressionQuality: 0.9) { context in
                UIColor.systemTeal.setFill()
                context.fill(CGRect(x: 0, y: 0, width: 1800, height: 2600))
            }.write(to: artifact)
        let before = HostMetrics.residentMB()
        let output = directory.appendingPathComponent("real.pdf")
        let started = Date()
        let (outcome, peak) = await HostMetrics.measure { try? await PdfExporter.compose(source: source, output: output) { $0 % 10 == 9 ? artifact : nil } }
        let pages = CGPDFDocument(output as CFURL)?.numberOfPages ?? 0
        print("EXPORTREAL pages=\(pages) translated=\(outcome?.translated ?? -1) seconds=\(Int(Date().timeIntervalSince(started))) rss+\(peak.rss - before)MB footprint=\(peak.footprint)MB in=\(ManagedFiles.size(of: source) / 1_048_576)MB out=\(ManagedFiles.size(of: output) / 1_048_576)MB")
        XCTAssertEqual(pages, CGPDFDocument(source as CFURL)?.numberOfPages)
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
/// 把 https://stub.test/... 的请求交给表里的应答, 不碰网络。
final class LinkStub: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var responses: [String: (status: Int, mime: String, body: Data)] = [:]

    override class func canInit(with request: URLRequest) -> Bool { request.url?.host == "stub.test" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let url = request.url!
        let entry = Self.responses[url.absoluteString] ?? (404, "text/plain", Data())
        let response = HTTPURLResponse(url: url, statusCode: entry.status, httpVersion: "HTTP/1.1", headerFields: ["Content-Type": entry.mime])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: entry.body)
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}

/// 一次导入的结果发给 TSX 什么 (NshHostViewController 把这些事件原样发出去)。
final class ImportDeliveryTests: XCTestCase {
    private func ref(_ name: String, type: String = "image/png") -> FileRef {
        FileRef(id: UUID().uuidString, name: name, size: 1, type: type)
    }

    func testImagesPdfAndIgnoredEachGetTheirOwnEvent() {
        let result = IngestResult(images: [ref("a.png"), ref("b.png")], pdfs: [ref("book.pdf", type: "application/pdf")], ignored: 3)
        let events = ImportDelivery(result).events()
        XCTAssertEqual(events.map(\.name), ["files.added", "pdf.added", "host.notice"])
        XCTAssertEqual((events[0].payload as? [[String: Any]])?.count, 2)
        XCTAssertEqual((events[2].payload as? [String: String])?["text"], "已忽略 3 个不是图片或 PDF 的文件")
    }

    func testOnlyTheFirstPDFReachesTheReaderAndTheRestAreNamedForRelease() {
        let first = ref("one.pdf", type: "application/pdf")
        let rest = [ref("two.pdf", type: "application/pdf"), ref("three.pdf", type: "application/pdf")]
        let delivery = ImportDelivery(IngestResult(pdfs: [first] + rest))
        XCTAssertEqual(delivery.droppedPDFs, rest)
        let events = delivery.events()
        XCTAssertEqual(events.map(\.name), ["pdf.added", "host.notice"])
        XCTAssertEqual((events[0].payload as? [[String: Any]])?.first?["id"] as? String, first.id)
        XCTAssertEqual((events[1].payload as? [String: String])?["text"], "一次只打开一个 PDF, 已忽略另外 2 个")
    }

    func testFilesPickKeepsImagesOutOfTheEventsBecauseTheyGoInTheReturnValue() {
        let delivery = ImportDelivery(IngestResult(images: [ref("a.png")], pdfs: [ref("book.pdf", type: "application/pdf")]))
        XCTAssertEqual(delivery.events(includeImages: false).map(\.name), ["pdf.added"])
    }

    func testNothingToSayMeansNoEvents() {
        XCTAssertTrue(ImportDelivery(IngestResult()).events().isEmpty)
    }
}

/// 剪贴板导入: 用私有的命名剪贴板, 读它不会触发系统的「允许粘贴」提示。
final class PasteboardIngestTests: XCTestCase {
    private var directory: URL!
    private var pasteboard: UIPasteboard!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("paste-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        pasteboard = try XCTUnwrap(UIPasteboard(name: UIPasteboard.Name("it-test-\(UUID().uuidString)"), create: true))
    }

    override func tearDownWithError() throws {
        UIPasteboard.remove(withName: pasteboard.name)
        try? FileManager.default.removeItem(at: directory)
    }

    private func ingestor() -> (FileIngestor, ManagedFiles) {
        let files = ManagedFiles(root: directory.appendingPathComponent("tmp"), pdfRoot: directory.appendingPathComponent("pdfs"))
        return (FileIngestor(files: files), files)
    }

    private func png(_ name: String) throws -> URL {
        let url = directory.appendingPathComponent(name)
        let renderer = UIGraphicsImageRenderer(size: CGSize(width: 8, height: 8))
        try renderer.pngData { context in
            UIColor.red.setFill()
            context.fill(CGRect(x: 0, y: 0, width: 8, height: 8))
        }.write(to: url)
        return url
    }

    private func pdf(_ name: String) throws -> URL {
        let url = directory.appendingPathComponent(name)
        try UIGraphicsPDFRenderer(bounds: CGRect(x: 0, y: 0, width: 100, height: 100)).writePDF(to: url) { $0.beginPage() }
        return url
    }

    func testCopiedImageFilesComeInAsFilesWithoutBeingDecoded() async throws {
        let first = try png("a.png")
        let second = try png("b.png")
        pasteboard.setItemProviders([DebugProvider.make(first), DebugProvider.make(second)].compactMap { $0 }, localOnly: true, expirationDate: nil)
        let (ingestor, files) = ingestor()
        let result = await ingestor.ingest(pasteboard: pasteboard, allowing: [.image, .pdf])
        XCTAssertEqual(result.images.count, 2)
        XCTAssertTrue(result.pdfs.isEmpty)
        for image in result.images {
            let stored = try XCTUnwrap(files.url(for: image.id))
            XCTAssertEqual(image.type, "image/png")
            XCTAssertGreaterThan(ManagedFiles.size(of: stored), 0)
        }
    }

    func testACopiedPDFIsRecognisedByItsType() async throws {
        pasteboard.setItemProviders([DebugProvider.make(try pdf("book.pdf"))].compactMap { $0 }, localOnly: true, expirationDate: nil)
        let (ingestor, files) = ingestor()
        let result = await ingestor.ingest(pasteboard: pasteboard, allowing: [.image, .pdf])
        let ref = try XCTUnwrap(result.pdfs.first)
        XCTAssertEqual(ref.pageCount, 1)
        XCTAssertNotNil(files.pdfURL(for: ref.id))
        XCTAssertTrue(result.images.isEmpty)
    }

    func testACopiedFileReferenceIsReadAsAFile() async throws {
        pasteboard.url = try png("ref.png")
        let (ingestor, _) = ingestor()
        let result = await ingestor.ingest(pasteboard: pasteboard, allowing: [.image, .pdf])
        XCTAssertEqual(result.images.map(\.name), ["ref.png"])
        XCTAssertTrue(FileManager.default.fileExists(atPath: directory.appendingPathComponent("ref.png").path), "a copied reference must not move the original")
    }

    func testTextOnlyClipboardIsEmptyAndNothingIsRead() async throws {
        pasteboard.string = "hello"
        XCTAssertFalse(FileIngestor.hasImportable(pasteboard))
        let (ingestor, _) = ingestor()
        let result = await ingestor.ingest(pasteboard: pasteboard, allowing: [.image, .pdf])
        XCTAssertTrue(ImportDelivery(result).isEmpty)
    }

    func testAWebLinkOnTheClipboardIsNotAnImageUntilItIsDownloaded() async throws {
        // 网页链接也算「有可读的东西」, 读下来才知道是不是图片; 这里不联网, 只确认它被当成链接处理而不是崩溃。
        pasteboard.url = URL(string: "http://127.0.0.1:9/never.png")
        XCTAssertTrue(FileIngestor.hasImportable(pasteboard))
    }
}
