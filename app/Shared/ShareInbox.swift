import Foundation
import UniformTypeIdentifiers

/// App 与 Share Extension 约定的收件箱。扩展只往里放文件, 不解码也不上传; App 在启动和每次回到前台时把它们搬走。
///
/// 布局 (App Group 容器下的 inbox/):
/// - `.staging/<uuid>/<文件>`: 扩展正在拷的文件, App 不看。拷完后整个目录改名成下面的条目, 改名是原子的,
///   所以 App 不会搬走只拷了一半的文件。
/// - `<毫秒>-<序号>-<uuid>/<文件>`: 一个条目, 目录里只有一个文件。目录名的字典序就是分享的先后, 同一次分享里
///   也是用户选择的顺序; 每个条目一个目录, 同名文件不会冲突。
enum ShareInbox {
    static let groupIdentifier = "group.com.wisdomworldworkshop.imagetranslator"
    /// 网页链接条目: 文件内容是 URL 文本。扩展有内存和时间上限, 不下载, 留给 App 导入时下载。
    static let linkExtension = "weblink"
    static let stagingName = ".staging"

    /// 收件箱目录。取不到 App Group 容器 (没有授权、没有注册) 时为 nil。
    static func directory(create: Bool = true) -> URL? {
        guard let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: groupIdentifier) else { return nil }
        let inbox = container.appendingPathComponent("inbox", isDirectory: true)
        if create { try? FileManager.default.createDirectory(at: inbox, withIntermediateDirectories: true) }
        return inbox
    }

    /// 拷完的条目目录, 按分享顺序。
    static func entries(in inbox: URL) -> [URL] {
        let names = (try? FileManager.default.contentsOfDirectory(atPath: inbox.path)) ?? []
        return names.filter { !$0.hasPrefix(".") }.sorted().map { inbox.appendingPathComponent($0, isDirectory: true) }
    }

    /// 扩展被系统杀掉时留下的暂存目录; 超过一小时的一定是死的, 删掉。
    static func removeStaleStaging(in inbox: URL, now: Date = Date()) {
        let staging = inbox.appendingPathComponent(stagingName, isDirectory: true)
        let keys: [URLResourceKey] = [.contentModificationDateKey]
        let leftovers = (try? FileManager.default.contentsOfDirectory(at: staging, includingPropertiesForKeys: keys)) ?? []
        for url in leftovers {
            let modified = (try? url.resourceValues(forKeys: Set(keys)))?.contentModificationDate
            if let modified, now.timeIntervalSince(modified) > 3600 { try? FileManager.default.removeItem(at: url) }
        }
    }

    /// 条目目录里要导入的文件 (目录里只有一个, 取第一个不是隐藏文件的)。
    static func payload(of entry: URL) -> URL? {
        let names = ((try? FileManager.default.contentsOfDirectory(atPath: entry.path)) ?? []).filter { !$0.hasPrefix(".") }.sorted()
        return names.first.map { entry.appendingPathComponent($0) }
    }
}

/// 导入的文件叫什么。分享、拖放和选择器给的临时副本常叫 "image.jpeg" 或 "PNG图像" 这类通用名, 用户在相册和文件里看到的名字在 provider.suggestedName 里,
/// 排序和识别都靠它, 所以优先用它; 扩展名取自实际的文件 (没有才取类型的), 免得把 HEIC 的内容叫成 .jpg。
enum FileNaming {
    static func name(for url: URL, suggested: String?, type: UTType) -> String {
        let actual = url.pathExtension
        let ext = actual.isEmpty ? (type.preferredFilenameExtension ?? "") : actual
        guard let suggested = cleaned(suggested) else {
            let base = url.lastPathComponent.isEmpty ? "file" : url.lastPathComponent
            return actual.isEmpty && !ext.isEmpty ? "\(base).\(ext)" : base
        }
        if ext.isEmpty || (suggested as NSString).pathExtension.caseInsensitiveCompare(ext) == .orderedSame { return suggested }
        return "\(suggested).\(ext)"
    }

    /// 去掉路径和不能放进文件名的字符; 空的返回 nil。
    static func cleaned(_ name: String?) -> String? {
        guard let name else { return nil }
        let base = (name as NSString).lastPathComponent.replacingOccurrences(of: ":", with: "_").trimmingCharacters(in: .whitespacesAndNewlines)
        return base.isEmpty || base == "." || base == ".." ? nil : base
    }
}

struct ShareDeposit: Equatable {
    var added = 0
    var skipped = 0
}

/// 把分享来的东西串行放进收件箱: 一次只有一个文件在拷, 扩展的内存上限 (约 120 MB) 因此与文件大小无关。
final class ShareDepositor: @unchecked Sendable {
    private let inbox: URL
    private let fileManager = FileManager.default

    init(inbox: URL) {
        self.inbox = inbox
    }

    func deposit(items: [NSExtensionItem], now: Date = Date()) async -> ShareDeposit {
        await deposit(items.flatMap { $0.attachments ?? [] }, now: now)
    }

    func deposit(_ providers: [NSItemProvider], now: Date = Date()) async -> ShareDeposit {
        let batch = Int(now.timeIntervalSince1970 * 1000)
        var summary = ShareDeposit()
        for (index, provider) in providers.enumerated() {
            let entry = String(format: "%013ld-%04ld-%@", batch, index, String(UUID().uuidString.prefix(8)))
            if await deposit(provider, entry: entry) { summary.added += 1 } else { summary.skipped += 1 }
        }
        return summary
    }

    private func deposit(_ provider: NSItemProvider, entry: String) async -> Bool {
        if let type = Self.fileType(of: provider) {
            let suggested = provider.suggestedName
            return await withCheckedContinuation { continuation in
                provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, _ in
                    guard let url else {
                        continuation.resume(returning: false)
                        return
                    }
                    continuation.resume(returning: self.place(url, type: type, suggested: suggested, entry: entry))
                }
            }
        }
        // loadObject 替我们解码: loadItem 给的 URL 有时是 NSURL, 有时是归档过的 Data。
        guard provider.canLoadObject(ofClass: URL.self) else { return false }
        return await withCheckedContinuation { continuation in
            _ = provider.loadObject(ofClass: URL.self) { link, _ in
                guard let link, Self.isImageLink(link) else {
                    continuation.resume(returning: false)
                    return
                }
                continuation.resume(returning: self.writeLink(link, entry: entry))
            }
        }
    }

    /// 系统给的临时文件在回调返回后就被删掉, 所以必须在回调里拷走; copyItem 按文件拷, 不把内容读进内存。
    private func place(_ source: URL, type: UTType, suggested: String?, entry: String) -> Bool {
        stage(entry) { directory in
            let name = FileNaming.name(for: source, suggested: suggested, type: type)
            try fileManager.copyItem(at: source, to: directory.appendingPathComponent(name))
        }
    }

    private func writeLink(_ link: URL, entry: String) -> Bool {
        stage(entry) { directory in
            try Data(link.absoluteString.utf8).write(to: directory.appendingPathComponent("link.\(ShareInbox.linkExtension)"))
        }
    }

    private func stage(_ entry: String, _ fill: (URL) throws -> Void) -> Bool {
        let staging = inbox.appendingPathComponent(ShareInbox.stagingName, isDirectory: true).appendingPathComponent(UUID().uuidString, isDirectory: true)
        do {
            try fileManager.createDirectory(at: staging, withIntermediateDirectories: true)
            try fill(staging)
            try fileManager.moveItem(at: staging, to: inbox.appendingPathComponent(entry, isDirectory: true))
            return true
        } catch {
            try? fileManager.removeItem(at: staging)
            return false
        }
    }

    // MARK: - 纯函数

    /// 要拷的那种表示: PDF 优先; 图片里挑 Daemon 认的格式 (PNG、JPEG、WebP 等), 都不是就取第一个图片类型。
    static func fileType(of provider: NSItemProvider) -> UTType? {
        let types = provider.registeredTypeIdentifiers.compactMap { UTType($0) }
        if let pdf = types.first(where: { $0.conforms(to: .pdf) }) { return pdf }
        let images = types.filter { $0.conforms(to: .image) }
        let preferred: [UTType] = [.png, .jpeg, .webP, .gif, .bmp, .tiff]
        for candidate in preferred where images.contains(where: { $0.conforms(to: candidate) }) {
            return images.first { $0.conforms(to: candidate) }
        }
        return images.first
    }

    /// 只收 http(s) 链接, 并拦下明显是网页的 (空路径、.html 等): 分享一个普通网页不该显示「已加入」。
    static func isImageLink(_ link: URL) -> Bool {
        guard link.scheme == "http" || link.scheme == "https" else { return false }
        let path = link.path
        if path.isEmpty || path == "/" { return false }
        return !["html", "htm", "shtml", "php", "asp", "aspx", "jsp"].contains((path as NSString).pathExtension.lowercased())
    }
}