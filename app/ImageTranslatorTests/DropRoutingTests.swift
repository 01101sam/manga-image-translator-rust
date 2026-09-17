import UniformTypeIdentifiers
import XCTest
@testable import ImageTranslator

final class DropRoutingTests: XCTestCase {
    func testImageDataSubmitsImage() {
        XCTAssertEqual(routeDrop(type: .image, hasData: true), .submitImage)
        XCTAssertEqual(routeDrop(type: .jpeg, hasData: true), .submitImage)
        XCTAssertEqual(routeDrop(type: .png, hasData: true), .submitImage)
    }

    func testPDFDataOpensReader() {
        XCTAssertEqual(routeDrop(type: .pdf, hasData: true), .openPDF)
    }

    func testFileURLImportsAndPDFFileURLStillImports() {
        XCTAssertEqual(routeDrop(type: .fileURL, hasData: false), .importURL)
        XCTAssertEqual(routeDrop(type: .fileURL, hasData: true), .importURL)
        XCTAssertEqual(routeImportedURL(URL(fileURLWithPath: "/tmp/page.pdf")), .openPDF)
        XCTAssertEqual(routeImportedURL(URL(fileURLWithPath: "/tmp/photo.jpg")), .importURL)
    }

    func testUnknownIsIgnored() {
        XCTAssertEqual(routeDrop(type: .plainText, hasData: true), .ignore)
        XCTAssertEqual(routeDrop(type: .mp3, hasData: true), .ignore)
        XCTAssertEqual(routeDrop(type: .image, hasData: false), .ignore)
        XCTAssertEqual(routeDrop(type: .pdf, hasData: false), .ignore)
    }
}
