import XCTest
@testable import ImageTranslator

/// 「打开方式」: Info.plist 的声明, 以及 onOpenURL 之后的中转与导入。
@MainActor
final class OpenURLTests: XCTestCase {
    func testTheAppDeclaresImagesAndPDFsAndAsksTheSystemForACopy() throws {
        let info = Bundle.main.infoDictionary ?? [:]
        XCTAssertEqual(info["LSSupportsOpeningDocumentsInPlace"] as? Bool, false, "in-place would hand us the user's own file, and the host moves what it imports")
        let types = (info["CFBundleDocumentTypes"] as? [[String: Any]] ?? []).flatMap { $0["LSItemContentTypes"] as? [String] ?? [] }
        XCTAssertEqual(Set(types), ["public.image", "com.adobe.pdf"])
    }

    func testURLsThatArriveBeforeTheHostIsUpAreHeldAndBatchedTogether() async throws {
        let router = OpenURLRouter()
        router.open(URL(fileURLWithPath: "/tmp/a.pdf"))
        router.open(URL(fileURLWithPath: "/tmp/b.pdf"))
        router.open(URL(string: "https://example.com/x.png")!)
        var batches: [[URL]] = []
        router.handler = { batches.append($0) }
        try await Task.sleep(nanoseconds: OpenURLRouter.batchDelay * 3)
        XCTAssertEqual(batches.map { $0.map(\.lastPathComponent) }, [["a.pdf", "b.pdf"]], "one batch, file URLs only")
        router.open(URL(fileURLWithPath: "/tmp/c.png"))
        try await Task.sleep(nanoseconds: OpenURLRouter.batchDelay * 3)
        XCTAssertEqual(batches.count, 2)
    }

    func testOnlyFilesInsideTheAppsOwnContainerAreMovedAndTheRestAreOnlyCopied() throws {
        let inbox = URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent("Documents/Inbox/a.pdf")
        XCTAssertTrue(NshHostViewController.isInOwnContainer(inbox))
        XCTAssertTrue(NshHostViewController.isInOwnContainer(URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("x.png")))
        XCTAssertFalse(NshHostViewController.isInOwnContainer(URL(fileURLWithPath: "/Users/someone/Documents/a.pdf")))
        XCTAssertFalse(NshHostViewController.isInOwnContainer(URL(fileURLWithPath: NSHomeDirectory() + "-evil/a.pdf")), "a sibling path that merely starts with the same text is not ours")
    }
}
