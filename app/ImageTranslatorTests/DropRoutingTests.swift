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

    func testWebURLToImage() {
        XCTAssertEqual(routeDrop(type: .url, hasData: false), .downloadWeb)
        let jpeg = Data([0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10])
        XCTAssertEqual(sniffDropBytes(jpeg, contentType: "image/jpeg"), .submitImage)
        XCTAssertEqual(sniffDropBytes(jpeg, contentType: nil), .submitImage)
    }

    func testWebURLToPDF() {
        let pdf = Data("%PDF-1.4\n%".utf8)
        XCTAssertEqual(sniffDropBytes(pdf, contentType: "application/pdf"), .openPDF)
        XCTAssertEqual(sniffDropBytes(pdf, contentType: nil), .openPDF)
        XCTAssertEqual(sniffDropBytes(Data([0x00, 0x01]), contentType: "application/pdf"), .openPDF)
    }

    func testAssetWithImageDataSubmits() {
        let asset = UTType(importedAs: "com.apple.photos.object")
        XCTAssertFalse(asset.conforms(to: .image))
        XCTAssertEqual(routeDrop(type: asset, hasData: true), .submitImage)
        XCTAssertEqual(routeDrop(type: asset, hasData: false), .ignore)
    }

    func testUnknownTypeIgnoresWithMessage() {
        switch routeDrop(type: .plainText, hasData: true) {
        case .ignoreWithMessage(let message):
            XCTAssertEqual(message, unrecognizedDropMessage(typeIdentifiers: [UTType.plainText.identifier]))
        default:
            XCTFail("plain text should ignore with the registered type list")
        }
        switch routeDrop(type: .mp3, hasData: true) {
        case .ignoreWithMessage(let message):
            XCTAssertTrue(message.contains(UTType.mp3.identifier), message)
        default:
            XCTFail("mp3 should ignore with the registered type list")
        }
        XCTAssertEqual(routeDrop(type: .image, hasData: false), .ignore)
        XCTAssertEqual(routeDrop(type: .pdf, hasData: false), .ignore)
    }

    func testDropResultBanner() {
        XCTAssertEqual(dropResultBanner(images: 1, openedPDF: false, typeIdentifiers: []), "已导入 1 张图片")
        XCTAssertEqual(dropResultBanner(images: 0, openedPDF: true, typeIdentifiers: []), "已打开 PDF")
        XCTAssertEqual(
            dropResultBanner(images: 0, openedPDF: false, typeIdentifiers: ["public.url", "public.html"]),
            unrecognizedDropMessage(typeIdentifiers: ["public.url", "public.html"])
        )
    }
}
