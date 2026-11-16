import CoreGraphics
import ImageIO
import UIKit
import UniformTypeIdentifiers
import XCTest
@testable import ImageTranslator

/// 指纹的算法层: 固定的像素 -> 固定的 256 位。client-ui/tests/fingerprint.test.ts 用同样的生成器和同样的期望值测 JS 一侧, 两边都过就是位级一致。
final class ImageFingerprintTests: XCTestCase {
    enum Kind { case ramp, checker, lcg }

    /// 与 fingerprint.test.ts 里的 pattern 逐字对应。
    static func pattern(_ kind: Kind, _ width: Int, _ height: Int) -> [UInt8] {
        var rgba = [UInt8](repeating: 255, count: width * height * 4)
        var state: UInt32 = 12345
        for y in 0..<height {
            for x in 0..<width {
                let index = (y * width + x) * 4
                switch kind {
                case .ramp:
                    rgba[index] = UInt8(255 - x * 255 / max(width - 1, 1))
                    rgba[index + 1] = UInt8(y * 255 / max(height - 1, 1))
                    rgba[index + 2] = UInt8((x + y) * 255 / max(width + height - 2, 1))
                case .checker:
                    let value: UInt8 = ((x / 4) + (y / 4)) % 2 == 0 ? 230 : 25
                    rgba[index] = value
                    rgba[index + 1] = value
                    rgba[index + 2] = value
                case .lcg:
                    for channel in 0..<3 {
                        state = (state &* 1103515245 &+ 12345) & 0x7fffffff
                        rgba[index + channel] = UInt8((state >> 16) & 0xff)
                    }
                }
            }
        }
        return rgba
    }

    static let golden: [(Kind, Int, Int, String)] = [
        (.ramp, 64, 48, "673656755673aeeb567536736d676ce73673756766e7366b756766b6e6ae6767"),
        (.checker, 100, 75, "cc883322cc8800003322cc883322cc8833220000cc883322cc883322cc88cc88"),
        (.lcg, 40, 30, "724bbb128ec86b2d8c624974ad24344dc9b84ee34d59d4a8ed6662a9b6c91d68"),
        (.ramp, 5, 3, "1224122412241224122412241224122412241224122412241224122412241224"),
        (.lcg, 1, 1, "0000000000000000000000000000000000000000000000000000000000000000"),
        (.checker, 17, 16, "1010101010101010010101010101010110101010101010100101010101010101"),
    ]

    func testTheGoldenPatternsHashToTheValuesTheWebSideAlsoProduces() {
        for (kind, width, height, expected) in Self.golden {
            let found = ImageFingerprint.hash(rgba: Self.pattern(kind, width, height), width: width, height: height)
            print("GOLDEN \(kind) \(width)x\(height) \(found)")
            XCTAssertEqual(found, expected, "\(kind) \(width)x\(height)")
        }
    }

    func testAFlatPictureHasNoDifferencesAndALeftToRightRampSetsEveryBit() {
        XCTAssertEqual(ImageFingerprint.hash(rgba: [UInt8](repeating: 200, count: 32 * 32 * 4), width: 32, height: 32), String(repeating: "0", count: 64))
        var ramp = [UInt8](repeating: 255, count: 17 * 16 * 4)
        for y in 0..<16 { for x in 0..<17 { for channel in 0..<3 { ramp[(y * 17 + x) * 4 + channel] = UInt8(255 - x * 10) } } }
        XCTAssertEqual(ImageFingerprint.hash(grid: ImageFingerprint.grayGrid(rgba: ramp, width: 17, height: 16)), String(repeating: "f", count: 64))
    }

    func testDistanceCountsDifferingBitsAndRejectsMalformedHashes() {
        let zero = String(repeating: "0", count: 64)
        XCTAssertEqual(ImageFingerprint.distance(zero, zero), 0)
        XCTAssertEqual(ImageFingerprint.distance(zero, String(repeating: "f", count: 64)), 256)
        XCTAssertEqual(ImageFingerprint.distance(zero, "1" + String(repeating: "0", count: 63)), 1)
        XCTAssertNil(ImageFingerprint.distance(zero, "abc"))
        XCTAssertNil(ImageFingerprint.distance(zero, String(repeating: "g", count: 64)))
    }

    /// 同一张图在不同分辨率和 JPEG 重压缩下的指纹, 以及画面不同的图的指纹 (实测的分布在 DESIGN.md)。这里只断言方向: 同图近, 异图远。
    func testTheSameImageAtAnotherSizeOrRecompressedIsCloseAndADifferentImageIsFar() throws {
        func render(_ width: Int, _ height: Int, _ draw: (CGContext, CGFloat, CGFloat) -> Void) -> CGImage {
            let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
            context.setFillColor(gray: 1, alpha: 1)
            context.fill(CGRect(x: 0, y: 0, width: width, height: height))
            draw(context, CGFloat(width), CGFloat(height))
            return context.makeImage()!
        }
        func pageA(_ c: CGContext, _ w: CGFloat, _ h: CGFloat) {
            c.setFillColor(gray: 0.1, alpha: 1)
            c.fill(CGRect(x: w * 0.1, y: h * 0.6, width: w * 0.5, height: h * 0.3))
            c.fill(CGRect(x: w * 0.55, y: h * 0.1, width: w * 0.35, height: h * 0.4))
            c.setFillColor(gray: 0.6, alpha: 1)
            c.fill(CGRect(x: w * 0.1, y: h * 0.1, width: w * 0.3, height: h * 0.4))
        }
        func pageB(_ c: CGContext, _ w: CGFloat, _ h: CGFloat) {
            c.setFillColor(gray: 0.1, alpha: 1)
            c.fill(CGRect(x: w * 0.5, y: h * 0.1, width: w * 0.4, height: h * 0.8))
            c.setFillColor(gray: 0.5, alpha: 1)
            c.fill(CGRect(x: w * 0.1, y: h * 0.3, width: w * 0.3, height: h * 0.2))
        }
        let big = ImageFingerprint.hash(of: render(800, 1100, pageA))!
        let small = ImageFingerprint.hash(of: render(260, 360, pageA))!
        let other = ImageFingerprint.hash(of: render(800, 1100, pageB))!
        XCTAssertLessThanOrEqual(try XCTUnwrap(ImageFingerprint.distance(big, small)), 12)
        XCTAssertGreaterThan(try XCTUnwrap(ImageFingerprint.distance(big, other)), 40)
    }
}