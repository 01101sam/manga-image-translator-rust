import UIKit
import XCTest
@testable import ImageTranslator

/// App Group 收件箱到沙盒: 用临时目录当收件箱, 不碰真正的 App Group 容器。
final class InboxDrainerTests: XCTestCase {
    private var directory: URL!
    private var inbox: URL!
    private var files: ManagedFiles!
    private var ingestor: FileIngestor!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("inbox-tests-\(UUID().uuidString)", isDirectory: true)
        inbox = directory.appendingPathComponent("inbox", isDirectory: true)
        try FileManager.default.createDirectory(at: inbox, withIntermediateDirectories: true)
        files = ManagedFiles(root: directory.appendingPathComponent("tmp"), pdfRoot: directory.appendingPathComponent("pdfs"))
        ingestor = FileIngestor(files: files)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: directory)
    }

    @discardableResult
    private func deposit(_ entry: String, _ name: String, _ data: Data = Data(repeating: 1, count: 64)) throws -> URL {
        let folder = inbox.appendingPathComponent(entry, isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let url = folder.appendingPathComponent(name)
        try data.write(to: url)
        return url
    }

    private func pdfData() throws -> Data {
        UIGraphicsPDFRenderer(bounds: CGRect(x: 0, y: 0, width: 100, height: 100)).pdfData { $0.beginPage() }
    }

    private func leftovers() -> [String] {
        ShareInbox.entries(in: inbox).map(\.lastPathComponent)
    }

    func testEveryEntryMovesIntoTheSandboxInSharingOrderAndTheInboxIsEmptied() async throws {
        let first = try deposit("0000000000001-0000-aaaaaaaa", "page2.png")
        try deposit("0000000000001-0001-bbbbbbbb", "page10.png")
        try deposit("0000000000002-0000-cccccccc", "book.pdf", try pdfData())
        let result = await InboxDrainer(inbox: inbox).drain(using: ingestor)
        XCTAssertEqual(result.images.map(\.name), ["page2.png", "page10.png"])
        XCTAssertEqual(result.pdfs.map(\.name), ["book.pdf"])
        XCTAssertEqual(result.pdfs.first?.pageCount, 1)
        XCTAssertEqual(result.ignored, 0)
        XCTAssertTrue(leftovers().isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: first.path))
        for image in result.images {
            let stored = try XCTUnwrap(files.url(for: image.id))
            XCTAssertFalse(stored.path.hasPrefix(inbox.path), "the file must be in the sandbox, not still in the inbox")
            XCTAssertEqual(ManagedFiles.size(of: stored), 64)
        }
    }

    func testDrainingAgainImportsNothingAndNothingIsDuplicated() async throws {
        try deposit("0000000000001-0000-aaaaaaaa", "a.png")
        let drainer = InboxDrainer(inbox: inbox)
        let first = await drainer.drain(using: ingestor)
        let second = await drainer.drain(using: ingestor)
        XCTAssertEqual(first.images.count, 1)
        XCTAssertTrue(ImportDelivery(second).isEmpty)
    }

    func testACrashAfterTheMoveLeavesAnEmptyEntryThatIsOnlyRemoved() async throws {
        try FileManager.default.createDirectory(at: inbox.appendingPathComponent("0000000000001-0000-dead0000"), withIntermediateDirectories: true)
        let result = await InboxDrainer(inbox: inbox).drain(using: ingestor)
        XCTAssertTrue(ImportDelivery(result).isEmpty)
        XCTAssertTrue(leftovers().isEmpty)
    }

    func testAFileStillBeingCopiedIntoStagingIsLeftAlone() async throws {
        let staging = inbox.appendingPathComponent(ShareInbox.stagingName, isDirectory: true).appendingPathComponent("half", isDirectory: true)
        try FileManager.default.createDirectory(at: staging, withIntermediateDirectories: true)
        try Data("partial".utf8).write(to: staging.appendingPathComponent("page.png"))
        let result = await InboxDrainer(inbox: inbox).drain(using: ingestor)
        XCTAssertTrue(ImportDelivery(result).isEmpty)
        XCTAssertTrue(FileManager.default.fileExists(atPath: staging.appendingPathComponent("page.png").path))
    }

    func testOtherKindsAreIgnoredAndStillCleanedOut() async throws {
        try deposit("0000000000001-0000-aaaaaaaa", "notes.txt")
        let result = await InboxDrainer(inbox: inbox).drain(using: ingestor)
        XCTAssertEqual(result.ignored, 1)
        XCTAssertTrue(result.images.isEmpty)
        XCTAssertTrue(leftovers().isEmpty, "an entry that cannot be imported must not come back at every launch")
    }

    func testWebLinkEntriesAreDownloadedLikeADroppedLink() async throws {
        let png = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0])
        LinkStub.responses = ["https://stub.test/img/page01": (200, "application/octet-stream", png)]
        URLProtocol.registerClass(LinkStub.self)
        defer { URLProtocol.unregisterClass(LinkStub.self) }
        try deposit("0000000000001-0000-aaaaaaaa", "link.\(ShareInbox.linkExtension)", Data("https://stub.test/img/page01\n".utf8))
        try deposit("0000000000001-0001-bbbbbbbb", "link.\(ShareInbox.linkExtension)", Data("ftp://stub.test/x.png".utf8))
        let result = await InboxDrainer(inbox: inbox).drain(using: ingestor)
        XCTAssertEqual(result.images.map(\.name), ["page01.png"])
        XCTAssertEqual(result.ignored, 1)
        XCTAssertTrue(leftovers().isEmpty)
    }

    func testTwoDrainsAtOnceImportEachFileExactlyOnce() async throws {
        for index in 0..<30 { try deposit(String(format: "0000000000001-%04d-aaaaaaaa", index), "p\(index).png") }
        let drainer = InboxDrainer(inbox: inbox)
        async let first = drainer.drain(using: ingestor)
        async let second = drainer.drain(using: ingestor)
        let all = await first.images + second.images
        XCTAssertEqual(all.count, 30)
        XCTAssertEqual(Set(all.map(\.id)).count, 30)
        XCTAssertEqual(Set(all.map(\.name)).count, 30)
    }

    func testWithoutAGroupContainerThereIsNothingToDo() async {
        let result = await InboxDrainer(inbox: nil).drain(using: ingestor)
        XCTAssertTrue(ImportDelivery(result).isEmpty)
    }
}
