import UniformTypeIdentifiers
import XCTest
@testable import ImageTranslator

/// 分享扩展放进收件箱的东西 (扩展本身只是 ShareDepositor 的一层壳, 它的逻辑在这里测)。
final class ShareInboxTests: XCTestCase {
    private var directory: URL!
    private var inbox: URL!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("share-tests-\(UUID().uuidString)", isDirectory: true)
        inbox = directory.appendingPathComponent("inbox", isDirectory: true)
        try FileManager.default.createDirectory(at: inbox, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: directory)
    }

    private func write(_ name: String, _ bytes: Data = Data("x".utf8)) throws -> URL {
        let url = directory.appendingPathComponent(name)
        try bytes.write(to: url)
        return url
    }

    private func names(in entries: [URL]) -> [String] {
        entries.compactMap { ShareInbox.payload(of: $0)?.lastPathComponent }
    }

    /// 系统把 provider 的文件交出来时可能换扩展名 (c.jpg 变 c.jpeg), 顺序和类型才是要紧的。
    private func stems(in entries: [URL]) -> [String] {
        names(in: entries).map { ($0 as NSString).deletingPathExtension }
    }

    func testEachSharedFileGetsItsOwnEntryInSelectionOrder() async throws {
        let providers = [
            try write("b.png", Data(repeating: 1, count: 4096)),
            try write("a.pdf", Data("%PDF-1.4".utf8)),
            try write("c.jpg"),
        ].compactMap(DebugProvider.make)
        let summary = await ShareDepositor(inbox: inbox).deposit(providers)
        XCTAssertEqual(summary, ShareDeposit(added: 3, skipped: 0))
        let entries = ShareInbox.entries(in: inbox)
        XCTAssertEqual(entries.count, 3)
        XCTAssertEqual(stems(in: entries), ["b", "a", "c"])
        XCTAssertEqual(names(in: entries).prefix(2).map { $0 }, ["b.png", "a.pdf"])
        let first = try XCTUnwrap(ShareInbox.payload(of: entries[0]))
        XCTAssertEqual(ManagedFiles.size(of: first), 4096)
        let staging = (try? FileManager.default.contentsOfDirectory(atPath: inbox.appendingPathComponent(ShareInbox.stagingName).path)) ?? []
        XCTAssertTrue(staging.isEmpty, "nothing may be left in staging after a clean run")
    }

    func testAHalfCopiedFileInStagingIsInvisibleToTheApp() throws {
        let staging = inbox.appendingPathComponent(ShareInbox.stagingName).appendingPathComponent("half", isDirectory: true)
        try FileManager.default.createDirectory(at: staging, withIntermediateDirectories: true)
        try Data("partial".utf8).write(to: staging.appendingPathComponent("page.png"))
        XCTAssertTrue(ShareInbox.entries(in: inbox).isEmpty)
    }

    func testStagingLeftBehindByAKilledExtensionIsRemovedOnlyWhenOld() throws {
        let staging = inbox.appendingPathComponent(ShareInbox.stagingName, isDirectory: true)
        let old = staging.appendingPathComponent("old", isDirectory: true)
        let fresh = staging.appendingPathComponent("fresh", isDirectory: true)
        for url in [old, fresh] { try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true) }
        try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSinceNow: -2 * 3600)], ofItemAtPath: old.path)
        ShareInbox.removeStaleStaging(in: inbox)
        XCTAssertFalse(FileManager.default.fileExists(atPath: old.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: fresh.path))
    }

    func testOnlyImagesPdfsAndImageLinksAreAccepted() async throws {
        let text = DebugProvider.make(try write("notes.txt"))
        let links = ["https://x.test/img/page01.png", "https://x.test/index.html", "https://x.test/", "ftp://x.test/a.png", "https://x.test/img?id=3"]
            .map { NSItemProvider(object: NSURL(string: $0)!) }
        let summary = await ShareDepositor(inbox: inbox).deposit([text].compactMap { $0 } + links)
        XCTAssertEqual(summary, ShareDeposit(added: 2, skipped: 4))
        let entries = ShareInbox.entries(in: inbox)
        XCTAssertEqual(names(in: entries), ["link.\(ShareInbox.linkExtension)", "link.\(ShareInbox.linkExtension)"])
        let texts = try entries.map { try String(contentsOf: XCTUnwrap(ShareInbox.payload(of: $0)), encoding: .utf8) }
        XCTAssertEqual(texts, ["https://x.test/img/page01.png", "https://x.test/img?id=3"])
    }

    func testTheProviderTypeThePhotoLibraryUsuallyOffersIsChosenForTheDaemon() {
        func provider(_ types: [UTType]) -> NSItemProvider {
            let provider = NSItemProvider()
            for type in types {
                provider.registerDataRepresentation(forTypeIdentifier: type.identifier, visibility: .all) { completion in
                    completion(Data(), nil)
                    return nil
                }
            }
            return provider
        }
        XCTAssertEqual(ShareDepositor.fileType(of: provider([.heic, .jpeg])), .jpeg)
        XCTAssertEqual(ShareDepositor.fileType(of: provider([.heic, .png, .jpeg])), .png)
        XCTAssertEqual(ShareDepositor.fileType(of: provider([.heic])), .heic)
        XCTAssertEqual(ShareDepositor.fileType(of: provider([.png, .pdf])), .pdf)
        XCTAssertNil(ShareDepositor.fileType(of: provider([.plainText])))
    }

    func testTheShareExtensionActivatesForImagesPdfsAndLinks() throws {
        let plist = try XCTUnwrap(Bundle.main.url(forResource: "Info", withExtension: "plist", subdirectory: "PlugIns/ShareExtension.appex"))
        let info = try XCTUnwrap(NSDictionary(contentsOf: plist) as? [String: Any])
        let extensionInfo = try XCTUnwrap(info["NSExtension"] as? [String: Any])
        XCTAssertEqual(extensionInfo["NSExtensionPointIdentifier"] as? String, "com.apple.share-services")
        let rule = try XCTUnwrap((extensionInfo["NSExtensionAttributes"] as? [String: Any])?["NSExtensionActivationRule"] as? String)
        for type in ["public.image", "com.adobe.pdf", "public.url"] { XCTAssertTrue(rule.contains(type), "activation rule misses \(type)") }
    }
}
