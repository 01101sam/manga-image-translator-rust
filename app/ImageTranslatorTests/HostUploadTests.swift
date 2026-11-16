import XCTest
@testable import ImageTranslator

final class HostUploadTests: XCTestCase {
    func testStatusCodesMapToTheProtocolKinds() {
        func kind(_ status: Int, _ json: String = "{}") -> UploadFailure? {
            if case .failed(let kind, _) = UploadOutcome.from(status: status, body: Data(json.utf8)) { return kind }
            return nil
        }
        XCTAssertEqual(kind(401), .unauthorized)
        XCTAssertEqual(kind(429), .queueFull)
        XCTAssertEqual(kind(503), .engineStopped)
        for status in [400, 413, 415, 422, 404] { XCTAssertEqual(kind(status), .rejected, "HTTP \(status)") }
        for status in [500, 502, 599] { XCTAssertEqual(kind(status), .network, "HTTP \(status)") }
        XCTAssertEqual(kind(200), .network)
    }

    func testAcceptedResponseCarriesTheJobID() {
        let outcome = UploadOutcome.from(status: 200, body: Data(#"{"job_id":"abc"}"#.utf8))
        XCTAssertEqual(outcome, .accepted(jobID: "abc"))
        XCTAssertEqual(outcome.json["jobId"] as? String, "abc")
    }

    func testFailureMessageComesFromTheDaemonWhenPresent() {
        let outcome = UploadOutcome.from(status: 429, body: Data(#"{"error":"queue full"}"#.utf8))
        XCTAssertEqual(outcome, .failed(.queueFull, message: "queue full"))
        XCTAssertEqual(outcome.json["kind"] as? String, "queue-full")
    }

    func testMultipartBodyHasBothFieldsAndTheExactFileBytes() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("upload-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        // 比一个分块 (1 MiB) 大, 且长度不是分块的整数倍, 才能测到分块边界。
        let payload = Data((0..<(Multipart.chunkSize * 2 + 4321)).map { UInt8(truncatingIfNeeded: $0 &* 31 &+ 7) })
        let file = directory.appendingPathComponent("a\"b.png")
        try payload.write(to: file)
        let body = directory.appendingPathComponent("body")
        try Multipart.writeBody(
            to: body, boundary: "B", file: file, filename: file.lastPathComponent, mime: "image/png", overrides: #"{"target_lang":"chs"}"#
        )
        let bytes = try Data(contentsOf: body)
        let head = Data("--B\r\nContent-Disposition: form-data; name=\"image\"; filename=\"a%22b.png\"\r\nContent-Type: image/png\r\n\r\n".utf8)
        let tail = Data("\r\n--B\r\nContent-Disposition: form-data; name=\"overrides\"\r\n\r\n{\"target_lang\":\"chs\"}\r\n--B--\r\n".utf8)
        XCTAssertEqual(bytes, head + payload + tail)
    }

    func testDaemonEndpointParsesOnlyHTTPAddresses() {
        let endpoint = DaemonEndpoint(name: "studio", url: "http://192.168.1.5:8080")
        XCTAssertEqual(endpoint, DaemonEndpoint(name: "studio", host: "192.168.1.5", port: 8080))
        XCTAssertEqual(DaemonEndpoint(name: "a", url: "http://studio.local")?.port, 80)
        XCTAssertNil(DaemonEndpoint(name: "a", url: "https://studio.local"))
        XCTAssertNil(DaemonEndpoint(name: "a", url: "file:///etc"))
        XCTAssertNil(DaemonEndpoint(name: "a", url: "http://h:99999"))
    }

    func testSnapshotShapesFollowTheProtocol() async throws {
        let paired = PairedDaemon(endpoint: DaemonEndpoint(name: "studio", host: "10.0.0.2", port: 8080), token: Token(raw: "tok"))
        let json = await MainActor.run { HostSession.json(paired.endpoint) }
        XCTAssertEqual(json["name"] as? String, "studio")
        XCTAssertEqual(json["url"] as? String, "http://10.0.0.2:8080")
    }

    func testTokenRoundTripsThroughSaveAndSnapshot() async throws {
        try await MainActor.run {
            let store = MemoryTokenStore()
            let session = HostSession(tokens: store)
            try session.saveToken(["daemon": ["name": "studio", "url": "http://10.0.0.2:8080"], "token": "t-1"])
            let snapshot = session.snapshot()
            let paired = try XCTUnwrap(snapshot["paired"] as? [String: Any])
            XCTAssertEqual(paired["token"] as? String, "t-1")
            XCTAssertEqual((paired["daemon"] as? [String: Any])?["url"] as? String, "http://10.0.0.2:8080")
            XCTAssertThrowsError(try session.saveToken(["daemon": ["name": "x", "url": "ftp://x"], "token": "t"]))
            session.clearToken()
            XCTAssertTrue(session.snapshot()["paired"] is NSNull)
        }
    }

    func testArtifactExtensionFollowsContentType() {
        XCTAssertEqual(ArtifactSaver.fileExtension(forContentType: "image/png"), "png")
        XCTAssertEqual(ArtifactSaver.fileExtension(forContentType: "image/jpeg; charset=binary"), "jpg")
        XCTAssertEqual(ArtifactSaver.fileExtension(forContentType: "image/webp"), "webp")
        XCTAssertEqual(ArtifactSaver.fileExtension(forContentType: "text/html"), "html")
        XCTAssertEqual(ArtifactSaver.fileExtension(forContentType: nil), "bin")
    }
}
