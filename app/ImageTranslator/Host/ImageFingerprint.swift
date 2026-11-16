import CoreGraphics
import Foundation

/// 相似图片的指纹: 256 位的差分哈希 (dHash)。导入时算一次, 跟着文件引用交给 TSX, 用来判断「同一张图换了分辨率或重新压缩」。
///
/// 算法 (Web 宿主的 host.js 逐行对应, 两边对同一份像素给出同一个哈希, 用同样的黄金样本测):
/// 1. 把图 (已经在白底上合成、方向已修正的 RGBA8) 缩成 17 列 16 行的灰度: 格子 (cx, cy) 取源像素 x 从 cx * 宽 / 17 到 (cx + 1) * 宽 / 17 (不含)、
///    y 从 cy * 高 / 16 到 (cy + 1) * 高 / 16 (不含) 这一块的灰度平均; 除法都是整数除法, 这一块至少一个像素 (宽或高小于格数时相邻格子共用像素)。
///    全部用整数, 没有浮点。灰度 = (77 R + 150 G + 29 B + 128) >> 8。每格 = (灰度之和 + 个数 / 2) / 个数。
/// 2. 每行 17 个值两两比较得 16 位, 行优先, 左边比右边亮 (严格大于, 再加 margin) 记 1; 共 16 x 16 = 256 位, 每 4 位一个十六进制字符,
///    最高位在前, 64 个小写字符。
/// 3. 距离 = 两个哈希不同的位数。
enum ImageFingerprint {
    static let columns = 17
    static let rows = 16
    static let hexLength = columns > 1 ? (columns - 1) * rows / 4 : 0

    /// 左边比右边亮多少灰度级才算 1。0 是最朴素的 dHash; 平坦区域 (纸的留白) 里的噪声会让这些位随机翻转, 留一点死区让它们稳定为 0。
    /// 3 是 scripts/fingerprint-study 实测选的 (同图的变体: 页 0 时最远 19 位、3 时 12 位, 近空白页 0 时 35 位、3 时 7 位; 不同的页最近 45 位, 见 client-ui/DESIGN.md)。
    static let margin = 3

    /// RGBA8 (4 字节一个像素, 行优先, 不透明) 缩成 17x16 的灰度。
    static func grayGrid(rgba: [UInt8], width: Int, height: Int) -> [UInt8] {
        var grid = [UInt8](repeating: 0, count: columns * rows)
        guard width > 0, height > 0, rgba.count >= width * height * 4 else { return grid }
        for cy in 0..<rows {
            let y0 = min(cy * height / rows, height - 1)
            let y1 = max((cy + 1) * height / rows, y0 + 1)
            for cx in 0..<columns {
                let x0 = min(cx * width / columns, width - 1)
                let x1 = max((cx + 1) * width / columns, x0 + 1)
                var sum = 0
                for y in y0..<y1 {
                    var index = (y * width + x0) * 4
                    for _ in x0..<x1 {
                        sum += (77 * Int(rgba[index]) + 150 * Int(rgba[index + 1]) + 29 * Int(rgba[index + 2]) + 128) >> 8
                        index += 4
                    }
                }
                let count = (y1 - y0) * (x1 - x0)
                grid[cy * columns + cx] = UInt8((sum + count / 2) / count)
            }
        }
        return grid
    }

    /// 17x16 灰度的 256 位差分哈希, 64 个十六进制字符。
    static func hash(grid: [UInt8]) -> String {
        var bits = [UInt8](repeating: 0, count: columns > 1 ? (columns - 1) * rows / 8 : 0)
        var position = 0
        for row in 0..<rows {
            for column in 0..<(columns - 1) {
                if Int(grid[row * columns + column]) > Int(grid[row * columns + column + 1]) + margin {
                    bits[position >> 3] |= UInt8(0x80 >> (position & 7))
                }
                position += 1
            }
        }
        return bits.map { String(format: "%02x", $0) }.joined()
    }

    static func hash(rgba: [UInt8], width: Int, height: Int) -> String {
        hash(grid: grayGrid(rgba: rgba, width: width, height: height))
    }

    /// 一幅解码好的图 (宿主已经按方向和大小处理过, 通常长边 256 像素) 的哈希。透明的部分按白底算, 和 TSX 里看到的一致。
    static func hash(of image: CGImage) -> String? {
        grid(of: image).map(hash(grid:))
    }

    /// 同上, 只到 17x16 灰度这一步 (实测工具要用它换不同的 margin)。
    static func grid(of image: CGImage) -> [UInt8]? {
        let width = image.width
        let height = image.height
        guard width > 0, height > 0 else { return nil }
        var pixels = [UInt8](repeating: 255, count: width * height * 4)
        let drawn: Bool = pixels.withUnsafeMutableBytes { buffer in
            guard let context = CGContext(
                data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
            ) else { return false }
            context.setFillColor(red: 1, green: 1, blue: 1, alpha: 1)
            context.fill(CGRect(x: 0, y: 0, width: width, height: height))
            context.interpolationQuality = .none
            context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
            return true
        }
        return drawn ? grayGrid(rgba: pixels, width: width, height: height) : nil
    }

    /// 两个哈希不同的位数; 不是 64 位十六进制的返回 nil。
    static func distance(_ a: String, _ b: String) -> Int? {
        guard a.utf8.count == hexLength, b.utf8.count == hexLength else { return nil }
        var total = 0
        for (x, y) in zip(a.utf8, b.utf8) {
            guard let p = nibble(x), let q = nibble(y) else { return nil }
            total += (p ^ q).nonzeroBitCount
        }
        return total
    }

    private static func nibble(_ byte: UInt8) -> Int? {
        switch byte {
        case 48...57: return Int(byte) - 48
        case 97...102: return Int(byte) - 87
        default: return nil
        }
    }
}