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
    }

    func testParseBrowsePort() {
        let hooks = TestHooks.parse(["-daemon-host", "10.0.0.2", "-daemon-port", "18320"])
        XCTAssertEqual(hooks.browseDaemon, DaemonEndpoint(name: "Daemon", host: "10.0.0.2", port: 18320))
    }

    func testResolveSampleJpegFromAppBundle() {
        let url = TestHooks().resolveFile("sample.jpg")
        XCTAssertNotNil(url, "sample.jpg must ship in the app bundle")
        XCTAssertGreaterThan((try? Data(contentsOf: url!))?.count ?? 0, 0)
    }

    func testParseAutoSubmitCommaPaths() {
        let hooks = TestHooks.parse(["-auto-submit-image", "a.jpg,b.jpg"])
        XCTAssertEqual(hooks.autoSubmitPaths, ["a.jpg", "b.jpg"])
    }

    func testParseAutoImportFolder() {
        let hooks = TestHooks.parse(["-auto-import-folder", "batch"])
        XCTAssertEqual(hooks.autoImportFolder, "batch")
        XCTAssertTrue(hooks.isolatesSession)
    }

    func testEmptyArgsStayInert() {
        let hooks = TestHooks.parse(["ImageTranslator"])
        XCTAssertNil(hooks.paired)
        XCTAssertNil(hooks.browseDaemon)
        XCTAssertNil(hooks.autoImportFolder)
        XCTAssertFalse(hooks.isolatesSession)
    }
}
