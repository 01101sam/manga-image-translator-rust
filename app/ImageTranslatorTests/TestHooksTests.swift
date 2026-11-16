import XCTest
@testable import ImageTranslator

final class TestHooksTests: XCTestCase {
    func testParsePairedTest() {
        let hooks = TestHooks.parse(["-paired-test", "127.0.0.1", "18520", "tok"])
        XCTAssertEqual(
            hooks.paired,
            PairedDaemon(
                endpoint: DaemonEndpoint(name: "Daemon", host: "127.0.0.1", port: 18520),
                token: Token(raw: "tok")
            )
        )
        XCTAssertTrue(hooks.isolatesSession)
    }

    func testParseBrowsePort() {
        let hooks = TestHooks.parse(["-daemon-host", "10.0.0.2", "-daemon-port", "18320"])
        XCTAssertEqual(hooks.browseDaemon, DaemonEndpoint(name: "Daemon", host: "10.0.0.2", port: 18320))
        XCTAssertFalse(hooks.isolatesSession)
    }

    func testParseAutoImportFolder() {
        let hooks = TestHooks.parse(["-auto-import-folder", "batch"])
        XCTAssertEqual(hooks.autoImportFolder, "batch")
        XCTAssertFalse(hooks.isolatesSession)
    }

    func testParseHostHooks() {
        let hooks = TestHooks.parse(["-pick-folder", "/tmp/imgs", "-pick-pdf", "/tmp/a.pdf", "-section", "reader", "-forget-pairing"])
        XCTAssertEqual(hooks.pickFolder, "/tmp/imgs")
        XCTAssertEqual(hooks.pickPDF, "/tmp/a.pdf")
        XCTAssertEqual(hooks.section, "reader")
        XCTAssertTrue(hooks.forgetPairing)
        XCTAssertFalse(TestHooks.parse([]).forgetPairing)
        let more = TestHooks.parse(["-bundle", "/tmp/a.js", "-debug-drag-source", "/tmp/imgs", "-auto-import-folder", "/tmp/imgs", "-auto-import-urls"])
        XCTAssertEqual(more.bundlePath, "/tmp/a.js")
        XCTAssertEqual(more.debugDragSource, "/tmp/imgs")
        XCTAssertTrue(more.autoImportViaURLs)
    }

    func testEmptyArgsStayInert() {
        let hooks = TestHooks.parse(["ImageTranslator"])
        XCTAssertNil(hooks.paired)
        XCTAssertNil(hooks.browseDaemon)
        XCTAssertNil(hooks.autoImportFolder)
        XCTAssertFalse(hooks.isolatesSession)
    }
}