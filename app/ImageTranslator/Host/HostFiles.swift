import CoreGraphics
import Foundation
import UniformTypeIdentifiers

/// 宿主发给 TSX 的文件引用, 与 HOST_PROTOCOL.md 的 { id, name, size, type, modifiedAt? } 同形。
struct FileRef: Equatable, Sendable {
    var id: String
    var name: String
    var size: Int
    var type: String
    var pageCount: Int?
    /// 文件的修改时间, 毫秒时间戳。来自导入时的原文件 (URLResourceValues), 读不到时为 nil, TSX 排序时把它放在末尾。
    var modifiedAt: Double?

    var json: [String: Any] {
        var value: [String: Any] = ["id": id, "name": name, "size": size, "type": type]
        if let pageCount { value["pageCount"] = pageCount }
        if let modifiedAt { value["modifiedAt"] = modifiedAt }
        return value
    }
}

/// 一次拖放 / 选择的结果: 图片和 PDF 分开, TSX 对它们走不同的事件。
struct IngestResult: Equatable, Sendable {
    var images: [FileRef] = []
    var pdfs: [FileRef] = []
    var ignored = 0

    mutating func merge(_ other: IngestResult) {
        images += other.images
        pdfs += other.pdfs
        ignored += other.ignored
    }
}

enum HostFileKind: Sendable, Hashable {
    case image
    case pdf

    init?(url: URL) {
        guard let type = UTType(filenameExtension: url.pathExtension) else { return nil }
        if type.conforms(to: .pdf) {
            self = .pdf
        } else if type.conforms(to: .image) {
            self = .image
        } else {
            return nil
        }
    }
}

/// 文件的修改时间信不信: 用户自己的文件 (文档选择器、文件夹、打开方式) 读什么就报什么; NSItemProvider (拖放、照片选择器、粘贴) 给的是
/// 系统的临时副本, 实测 (模拟器, 进程内 provider) 副本的修改时间是复制的那一刻, 而不是原文件的, 所以刚刚才改过的时间当作没有;
/// 下载下来的链接和渲染出来的页只有生成时间, 一律没有。
enum FileDating: Sendable {
    case original
    case providerCopy
    case none

    /// 离导入这一刻不到这么久的修改时间, 对 provider 的副本来说分不清是文件真的刚改过还是副本刚生成, 一律当没有。
    static let justCopied: TimeInterval = 120
}

/// 面向用户的错误。宿主调用 reject 时 TSX 直接把 message 当 toast 显示。
struct HostCallError: Error, LocalizedError, Equatable {
    var message: String
    var errorDescription: String? { message }
    init(_ message: String) { self.message = message }
}

/// 宿主持有的文件: id 到沙盒内文件。文件内容不经过 JS, 宿主自己也不把它读进内存。
///
/// 图片 (拖放、选择、PDF 渲染出来的页) 放在 tmp 下, 启动时整个清掉, 因为 TSX 的待上传列表不跨启动保留。
/// PDF 放在 Application Support, id 就是文件名里的 uuid, 重启后仍然能解析。
final class ManagedFiles: @unchecked Sendable {
    static let pdfMaxAge: TimeInterval = 30 * 24 * 3600

    private let root: URL
    private let images: URL
    private let scratch: URL
    private let pdfRoot: URL
    private let lock = NSLock()
    private var registry: [String: URL] = [:]
    private var fileManager: FileManager { .default }

    init(root: URL, pdfRoot: URL) {
        self.root = root
        self.images = root.appendingPathComponent("images", isDirectory: true)
        self.scratch = root.appendingPathComponent("scratch", isDirectory: true)
        self.pdfRoot = pdfRoot
        prepareDirectories()
    }

    static func standard() -> ManagedFiles {
        let manager = FileManager.default
        let tmp = manager.temporaryDirectory.appendingPathComponent("imagetranslator-host", isDirectory: true)
        let support = manager.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("imagetranslator-host", isDirectory: true)
            .appendingPathComponent("pdfs", isDirectory: true)
        return ManagedFiles(root: tmp, pdfRoot: support)
    }

    private func prepareDirectories() {
        for directory in [images, scratch, pdfRoot] {
            try? fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        }
    }

    /// 启动时调用: 删掉上次运行遗留的临时文件, 以及超过 pdfMaxAge 没被打开过的 PDF。
    func cleanOnLaunch(now: Date = Date()) {
        lock.lock()
        registry.removeAll()
        lock.unlock()
        try? fileManager.removeItem(at: root)
        prepareDirectories()
        let entries = (try? fileManager.contentsOfDirectory(at: pdfRoot, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        for entry in entries {
            let modified = (try? entry.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate
            if let modified, now.timeIntervalSince(modified) > Self.pdfMaxAge {
                try? fileManager.removeItem(at: entry)
            }
        }
    }

    // MARK: - 图片 (含渲染出的 PDF 页)

    /// 把一个文件收进沙盒并登记。move 为真时搬走原文件 (拖放给的临时文件在回调返回后就会被系统删掉), 失败退回拷贝。
    func adopt(_ source: URL, name: String? = nil, move: Bool, dating: FileDating = .original, now: Date = Date()) throws -> FileRef {
        // 搬走之后原文件就没了, 所以修改时间要在搬之前读 (只读元数据, 不读内容)。
        let modifiedAt = Self.modifiedAt(of: source, dating: dating, now: now)
        let id = UUID().uuidString
        let directory = images.appendingPathComponent(id, isDirectory: true)
        try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        let fileName = Self.safeFileName(name ?? source.lastPathComponent, fallback: "image")
        let destination = directory.appendingPathComponent(fileName)
        do {
            try place(source, at: destination, move: move)
        } catch {
            try? fileManager.removeItem(at: directory)
            throw error
        }
        lock.lock()
        registry[id] = destination
        lock.unlock()
        return FileRef(id: id, name: fileName, size: Self.size(of: destination), type: Self.mimeType(of: destination), modifiedAt: modifiedAt)
    }

    func url(for id: String) -> URL? {
        lock.lock()
        defer { lock.unlock() }
        return registry[id]
    }

    func release(_ ids: [String]) {
        for id in ids {
            lock.lock()
            let file = registry.removeValue(forKey: id)
            lock.unlock()
            if let file { try? fileManager.removeItem(at: file.deletingLastPathComponent()) }
        }
    }

    /// 一个不登记的临时文件路径 (上传的 multipart 体、下载的 Artifact、渲染中的页), 用完由调用方删。
    func scratchFile(named name: String) -> URL {
        scratch.appendingPathComponent("\(UUID().uuidString)-\(Self.safeFileName(name, fallback: "file"))")
    }

    /// 要交给系统分享面板的文件: 面板和「存储到文件」显示的就是文件名, 所以名字不带前缀, 重名靠独立的子目录避免。
    func sharedFile(named name: String) -> URL {
        let directory = scratch.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try? fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory.appendingPathComponent(Self.safeFileName(name, fallback: "file"))
    }

    // MARK: - PDF

    func importPDF(from source: URL, name: String? = nil, move: Bool) throws -> FileRef {
        let id = UUID().uuidString
        let destination = pdfRoot.appendingPathComponent("\(id).pdf")
        try place(source, at: destination, move: move)
        let fileName = Self.safeFileName(name ?? source.lastPathComponent, fallback: "document.pdf")
        try? Data(fileName.utf8).write(to: nameFile(id))
        return FileRef(
            id: id, name: fileName, size: Self.size(of: destination), type: "application/pdf",
            pageCount: Self.pageCount(of: destination)
        )
    }

    /// id 来自 TSX, 先按 uuid 校验再拼路径, 防止 ../ 之类的路径。每次解析顺带刷新修改时间, 常用的 PDF 不会被启动清理删掉。
    func pdfURL(for id: String) -> URL? {
        guard UUID(uuidString: id) != nil else { return nil }
        let url = pdfRoot.appendingPathComponent("\(id).pdf")
        guard fileManager.fileExists(atPath: url.path) else { return nil }
        try? fileManager.setAttributes([.modificationDate: Date()], ofItemAtPath: url.path)
        return url
    }

    /// 重启后 TSX 凭上次的 id 找回 PDF: 文件还在就返回和 pdf.pick 一样的引用, 没有了返回 nil。
    func pdfRef(for id: String) -> FileRef? {
        guard let url = pdfURL(for: id) else { return nil }
        return FileRef(id: id, name: pdfDisplayName(for: id), size: Self.size(of: url), type: "application/pdf", pageCount: Self.pageCount(of: url))
    }

    func pdfDisplayName(for id: String) -> String {
        guard UUID(uuidString: id) != nil,
              let data = try? Data(contentsOf: nameFile(id)),
              let name = String(data: data, encoding: .utf8), !name.isEmpty
        else { return "document.pdf" }
        return name
    }

    func releasePDF(_ id: String) {
        guard UUID(uuidString: id) != nil else { return }
        try? fileManager.removeItem(at: pdfRoot.appendingPathComponent("\(id).pdf"))
        try? fileManager.removeItem(at: nameFile(id))
    }

    private func nameFile(_ id: String) -> URL {
        pdfRoot.appendingPathComponent("\(id).name")
    }

    // MARK: - 纯函数

    private func place(_ source: URL, at destination: URL, move: Bool) throws {
        if move {
            do {
                try fileManager.moveItem(at: source, to: destination)
                return
            } catch {
                try? fileManager.removeItem(at: destination)
            }
        }
        try fileManager.copyItem(at: source, to: destination)
    }

    static func modifiedAt(of url: URL, dating: FileDating, now: Date) -> Double? {
        guard dating != .none, let modified = modificationTime(of: url) else { return nil }
        if dating == .providerCopy, abs(now.timeIntervalSince1970 * 1000 - modified) < FileDating.justCopied * 1000 { return nil }
        return modified
    }

    /// 文件的修改时间 (毫秒时间戳); 读不到、或不是 1970 年之后的时间时返回 nil。
    static func modificationTime(of url: URL) -> Double? {
        guard let date = (try? url.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate else { return nil }
        let milliseconds = (date.timeIntervalSince1970 * 1000).rounded()
        return milliseconds > 0 ? milliseconds : nil
    }

    static func size(of url: URL) -> Int {
        ((try? FileManager.default.attributesOfItem(atPath: url.path))?[.size] as? NSNumber)?.intValue ?? 0
    }

    static func mimeType(of url: URL) -> String {
        UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? "application/octet-stream"
    }

    static func pageCount(of url: URL) -> Int? {
        CGPDFDocument(url as CFURL).map { $0.numberOfPages }
    }

    /// 去掉路径分隔符和控制字符, 留一个能直接放进 multipart 头和文件系统的名字。
    static func safeFileName(_ name: String, fallback: String) -> String {
        let base = (name as NSString).lastPathComponent
        let cleaned = String(base.unicodeScalars.filter { !CharacterSet.controlCharacters.contains($0) && $0 != "/" && $0 != ":" })
            .trimmingCharacters(in: .whitespaces)
        if cleaned.isEmpty || cleaned == "." || cleaned == ".." { return fallback }
        return String(cleaned.prefix(160))
    }
}