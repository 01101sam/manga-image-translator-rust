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
        XCTAssertEqual(summary, ShareDeposit(added: 3))
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
        XCTAssertEqual(summary, ShareDeposit(added: 2, skips: ShareSkips(unsupported: 4, unreadable: 0)))
        let entries = ShareInbox.entries(in: inbox)
        // 没加进来的数目作为这一批的最后一个条目留给 App。
        XCTAssertEqual(names(in: entries), ["link.\(ShareInbox.linkExtension)", "link.\(ShareInbox.linkExtension)", "skipped.\(ShareInbox.noteExtension)"])
        let texts = try entries.dropLast().map { try String(contentsOf: XCTUnwrap(ShareInbox.payload(of: $0)), encoding: .utf8) }
        XCTAssertEqual(texts, ["https://x.test/img/page01.png", "https://x.test/img?id=3"])
        let note = try JSONDecoder().decode(ShareSkips.self, from: Data(contentsOf: XCTUnwrap(ShareInbox.payload(of: entries.last!))))
        XCTAssertEqual(note, ShareSkips(unsupported: 4, unreadable: 0))
    }

    /// 用户的真机场景: 相册里有的照片还在 iCloud 里, 系统交不出文件。它们和「不是图片」是两回事, 提示里要分开说。
    func testPhotosThatCannotBeLoadedAreCountedAsUnreadableAndTheReasonsAreListedSeparately() async throws {
        let broken = NSItemProvider()
        broken.registerFileRepresentation(forTypeIdentifier: UTType.jpeg.identifier, fileOptions: [], visibility: .all) { completion in
            completion(nil, false, NSError(domain: NSURLErrorDomain, code: NSURLErrorNotConnectedToInternet))
            return nil
        }
        let fine = try XCTUnwrap(DebugProvider.make(try write("ok.png")))
        let text = try XCTUnwrap(DebugProvider.make(try write("notes.txt")))
        let summary = await ShareDepositor(inbox: inbox).deposit([fine, broken, text, broken])
        XCTAssertEqual(summary, ShareDeposit(added: 1, skips: ShareSkips(unsupported: 1, unreadable: 2)))
        XCTAssertEqual(summary.skipped, 3)
        XCTAssertEqual(summary.skips.reasons.count, 2)
        XCTAssertTrue(summary.skips.reasons[0].contains("1 项不是图片或 PDF"))
        XCTAssertTrue(summary.skips.reasons[1].contains("2 项没能读出来") && summary.skips.reasons[1].contains("iCloud"))
    }

    func testNothingIsLeftBehindForANoteWhenEverythingWasAdded() async throws {
        let summary = await ShareDepositor(inbox: inbox).deposit([try XCTUnwrap(DebugProvider.make(try write("a.png")))])
        XCTAssertEqual(summary.skipped, 0)
        XCTAssertEqual(names(in: ShareInbox.entries(in: inbox)).filter { $0.hasSuffix(ShareInbox.noteExtension) }, [])
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