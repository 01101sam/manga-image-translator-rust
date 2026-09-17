import XCTest
@testable import ImageTranslator

final class PairingMachineTests: XCTestCase {
    private let endpoint = DaemonEndpoint(name: "studio", host: "127.0.0.1", port: 8080)

    func testSelectRequestsPairThenAwaitsCode() {
        var phase = PairingPhase.browsing
        var command: PairingCommand
        (phase, command) = reducePairing(phase, .select(endpoint))
        XCTAssertEqual(phase, .requesting(endpoint))
        XCTAssertEqual(command, .requestPair(endpoint))
        (phase, command) = reducePairing(phase, .requestSucceeded)
        XCTAssertEqual(phase, .awaitingCode(endpoint))
        XCTAssertEqual(command, .none)
    }

    func testSixDigitCodeConfirmsAndPersistsToken() {
        var phase = PairingPhase.awaitingCode(endpoint)
        var command: PairingCommand
        (phase, command) = reducePairing(phase, .submitCode("123456"))
        XCTAssertEqual(phase, .confirming(endpoint, code: "123456"))
        XCTAssertEqual(command, .confirm(endpoint, code: "123456"))
        let token = Token(raw: "tok-1")
        (phase, command) = reducePairing(phase, .confirmSucceeded(token))
        XCTAssertEqual(phase, .paired(endpoint, token))
        XCTAssertEqual(command, .persist(PairedDaemon(endpoint: endpoint, token: token)))
    }

    func testRejectsShortCodeWithoutNetwork() {
        let phase = PairingPhase.awaitingCode(endpoint)
        let (next, command) = reducePairing(phase, .submitCode("12"))
        XCTAssertEqual(next, .failed(endpoint, message: "请输入6位数字"))
        XCTAssertEqual(command, .none)
    }

    func testConfirmFailureThenRetryRequestsAgain() {
        var phase = PairingPhase.confirming(endpoint, code: "000000")
        var command: PairingCommand
        (phase, command) = reducePairing(phase, .confirmFailed("wrong pairing code"))
        XCTAssertEqual(phase, .failed(endpoint, message: "wrong pairing code"))
        (phase, command) = reducePairing(phase, .retry)
        XCTAssertEqual(phase, .requesting(endpoint))
        XCTAssertEqual(command, .requestPair(endpoint))
    }

    func testUnauthorizedClearsToken() {
        let phase = PairingPhase.paired(endpoint, Token(raw: "old"))
        let (next, command) = reducePairing(phase, .unauthorized)
        XCTAssertEqual(next, .unauthorized(endpoint))
        XCTAssertEqual(command, .clearToken)
    }

    func testPairingFlowWithMockedSessionStoresToken() async throws {
        let store = MemoryTokenStore()
        MockURLProtocol.handler = { request in
            let path = request.url?.path ?? ""
            if path == "/pair/request" {
                return (200, Data(#"{"ok":true}"#.utf8))
            }
            if path == "/pair/confirm" {
                let body = request.httpBody.flatMap { try? JSONSerialization.jsonObject(with: $0) } as? [String: Any]
                XCTAssertEqual(body?["code"] as? String, "654321")
                return (200, Data(#"{"token":"secret-token"}"#.utf8))
            }
            if path == "/jobs" {
                XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer secret-token")
                return (200, Data(#"{"jobs":[]}"#.utf8))
            }
            return (404, Data(#"{"error":"missing"}"#.utf8))
        }
        defer { MockURLProtocol.handler = nil }

        let client = DaemonClient(
            session: MockURLProtocol.makeSession(),
            endpoint: endpoint,
            token: nil
        )
        try await client.requestPair()
        let token = try await client.confirm(code: "654321")
        XCTAssertEqual(token, Token(raw: "secret-token"))
        store.save(PairedDaemon(endpoint: endpoint, token: token))
        XCTAssertEqual(store.load()?.token.raw, "secret-token")

        let authed = DaemonClient(
            session: MockURLProtocol.makeSession(),
            endpoint: endpoint,
            token: token
        )
        let jobs = try await authed.listJobs()
        XCTAssertTrue(jobs.isEmpty)
    }

    func testConfirmFailureFromMockedSession() async {
        MockURLProtocol.handler = { request in
            if request.url?.path == "/pair/confirm" {
                return (401, Data(#"{"error":"wrong pairing code"}"#.utf8))
            }
            return (200, Data(#"{"ok":true}"#.utf8))
        }
        defer { MockURLProtocol.handler = nil }
        let client = DaemonClient(
            session: MockURLProtocol.makeSession(),
            endpoint: endpoint,
            token: nil
        )
        do {
            _ = try await client.confirm(code: "000000")
            XCTFail("confirm should fail")
        } catch {
            XCTAssertEqual(error as? DaemonError, .unauthorized)
        }
    }
}
