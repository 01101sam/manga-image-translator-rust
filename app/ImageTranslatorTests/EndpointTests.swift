import XCTest
@testable import ImageTranslator

final class EndpointTests: XCTestCase {
    func testIPv4ZoneSuffixYieldsHTTPURL() {
        let endpoint = DaemonEndpoint(name: "d", host: "192.168.100.3%en0", port: 8080)
        XCTAssertEqual(endpoint.baseURL.absoluteString, "http://192.168.100.3:8080")
    }

    func testIPv6ZoneSuffixYieldsBracketedHTTPURL() {
        let endpoint = DaemonEndpoint(name: "d", host: "fe80::1%en0", port: 8080)
        XCTAssertEqual(endpoint.baseURL.absoluteString, "http://[fe80::1]:8080")
    }

    func testBonjourNameHostYieldsHTTPURL() {
        let endpoint = DaemonEndpoint(name: "d", host: "macbook.local", port: 8080)
        XCTAssertEqual(endpoint.baseURL.absoluteString, "http://macbook.local:8080")
    }
}
