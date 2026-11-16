import Foundation
import ImageIO

// 指纹的实测工具: 读一个路径清单 (一行一个), 对每个文件用和 App 一样的解码 (ImagePrep.reducedImage, 长边 256) 和 ImageFingerprint 取 17x16 灰度,
// 输出 JSON 行 {"path", "hash", "width", "height", "grid"}。grid 让 study.py 能换不同的 margin 重新算哈希; hash 是 margin 为 App 当前值时的结果。
// 由 study.py 连同 App 的 HostImages.swift、ImageFingerprint.swift 一起编译, 所以量的就是 App 里跑的那份代码。
guard CommandLine.arguments.count == 2, let text = try? String(contentsOfFile: CommandLine.arguments[1], encoding: .utf8) else {
    FileHandle.standardError.write(Data("usage: fingerprint-study <paths.txt>\n".utf8))
    exit(2)
}
print("{\"margin\": \(ImageFingerprint.margin)}")
for path in text.split(separator: "\n").map(String.init) where !path.isEmpty {
    let url = URL(fileURLWithPath: path)
    guard let inspection = ImagePrep.inspect(at: url), let hash = inspection.hash,
          let source = CGImageSourceCreateWithURL(url as CFURL, nil),
          let reduced = ImagePrep.reducedImage(from: source, edge: ImagePrep.thumbnailEdge),
          let grid = ImageFingerprint.grid(of: reduced), ImageFingerprint.hash(grid: grid) == hash
    else {
        print("{\"path\": \(String(reflecting: path)), \"error\": true}")
        continue
    }
    print("{\"path\": \(String(reflecting: path)), \"hash\": \"\(hash)\", \"width\": \(inspection.width), \"height\": \(inspection.height), \"grid\": \(grid.map(String.init).joined(separator: ",").wrapped)}")
}

private extension String {
    var wrapped: String { "[" + self + "]" }
}