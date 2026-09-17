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

    func testPngMagicLabeledAsPng() {
        var bytes = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        bytes.append(contentsOf: [0x00, 0x00, 0x00, 0x0D])
        XCTAssertEqual(imageUploadLabel(for: bytes), ImageUploadLabel(filename: "photo.png", mime: "image/png"))
    }

    func testJpegMagicLabeledAsJpeg() {
        let bytes = Data([0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10])
        XCTAssertEqual(imageUploadLabel(for: bytes), ImageUploadLabel(filename: "photo.jpg", mime: "image/jpeg"))
    }

    func testUnknownBytesStayUnlabeled() {
        XCTAssertEqual(
            imageUploadLabel(for: Data([0x00, 0x01, 0x02])),
            ImageUploadLabel(filename: "photo.bin", mime: "application/octet-stream")
        )
    }

    func testEmptyArgsStayInert() {
        let hooks = TestHooks.parse(["ImageTranslator"])
        XCTAssertNil(hooks.paired)
        XCTAssertNil(hooks.browseDaemon)
        XCTAssertNil(hooks.autoImportFolder)
        XCTAssertFalse(hooks.isolatesSession)
    }
}
