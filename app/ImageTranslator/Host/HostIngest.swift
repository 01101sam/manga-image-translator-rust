import Foundation
import UIKit
import UniformTypeIdentifiers

/// 限制同时在途的任务数。一次拖放可能带来几百个 NSItemProvider, 不能同时展开。
actor AsyncLimiter {
    private let limit: Int
    private var running = 0
    private var waiters: [CheckedContinuation<Void, Never>] = []

    init(limit: Int) { self.limit = limit }

    func acquire() async {
        if running < limit {
            running += 1
            return
        }
        await withCheckedContinuation { waiters.append($0) }
    }

    func release() {
        if waiters.isEmpty {
            running -= 1
        } else {
            waiters.removeFirst().resume()
        }
    }

    func run<T: Sendable>(_ body: @Sendable () async -> T) async -> T {
        await acquire()
        let value = await body()
        release()
        return value
    }
}

/// 宿主要发给 TSX 的一个事件。
struct HostEvent {
    let name: String
    let payload: Any
}

/// 一次导入的结果该发给 TSX 什么。图片走 files.added (files.pick 则放在返回值里), PDF 走 pdf.added,
/// 其余给一条提示。阅读器一次只开一个 PDF: 多出来的不交给 TSX, 宿主立刻放掉它们在沙盒里的副本。
struct ImportDelivery {
    let images: [FileRef]
    let pdf: FileRef?
    let droppedPDFs: [FileRef]
    let ignored: Int

    /// 什么都没有: 没图片、没 PDF、也没有被忽略的东西。粘贴时据此告诉用户剪贴板是空的。
    var isEmpty: Bool { images.isEmpty && pdf == nil && ignored == 0 }

    init(_ result: IngestResult) {
        images = result.images
        pdf = result.pdfs.first
        droppedPDFs = Array(result.pdfs.dropFirst())
        ignored = result.ignored
    }

    func events(includeImages: Bool = true) -> [HostEvent] {
        var events: [HostEvent] = []
        if includeImages, !images.isEmpty { events.append(HostEvent(name: "files.added", payload: images.map(\.json))) }
        if let pdf { events.append(HostEvent(name: "pdf.added", payload: [pdf.json])) }
        if !droppedPDFs.isEmpty {
            events.append(HostEvent(name: "host.notice", payload: ["text": "一次只打开一个 PDF, 已忽略另外 \(droppedPDFs.count) 个"]))
        }
        if ignored > 0 { events.append(HostEvent(name: "host.notice", payload: ["text": "已忽略 \(ignored) 个不是图片或 PDF 的文件"])) }
        return events
    }
}

/// NSItemProvider 没有标 Sendable, 但读取它的回调本来就是跨线程设计的。
struct ProviderBox: @unchecked Sendable {
    let value: NSItemProvider
    init(_ value: NSItemProvider) { self.value = value }
}

/// 把拖放 / 选择得到的东西变成沙盒里的文件。
/// 每个 NSItemProvider 只读一种表示 (PDF 或图片, 不再试第二种), 并发读取不超过 limit 个。
final class FileIngestor: @unchecked Sendable {
    /// 一次只读一个。iPad 模拟器里进程内 provider 同时读两个以上时, RSS 涨到读过的总字节数左右: 30 张 37 MB 的 PNG, 并发 2 和 4 的 RSS 峰值都是
    /// 1.37 GB, 并发 1 是 332 MB; 200 张 1.7 MB 的 JPEG 并发 1 和 4 的耗时都是 0.9 秒。phys_footprint 始终是 23 到 24 MB。
    /// vmmap 显示多出来的常驻内存在只读的系统库区域 (__LINKEDIT、__TEXT), 不在文件映射里 (mapped file 只有 11 MB 常驻), 机制没查清, 也只在模拟器里量过。
    /// 限流要罩住整个 load, 连 hasItemConformingToTypeIdentifier 这样的类型判断也在内: 只限住读文件的那一步, 同样的测试 RSS 又涨回 240 MB。
    /// 代价是下载网页链接也占这一个名额, 一个慢链接会拖住后面的拖放和选择。
    static let concurrency = 1

    private let files: ManagedFiles
    private let limiter: AsyncLimiter

    init(files: ManagedFiles, limit: Int = FileIngestor.concurrency) {
        self.files = files
        self.limiter = AsyncLimiter(limit: limit)
    }

    func ingest(providers: [NSItemProvider], allowing kinds: Set<HostFileKind>) async -> IngestResult {
        let parts = await withTaskGroup(of: (Int, IngestResult).self) { group in
            for (index, provider) in providers.map(ProviderBox.init).enumerated() {
                group.addTask { [self] in (index, await limiter.run { await load(provider.value, allowing: kinds) }) }
            }
            var collected: [(Int, IngestResult)] = []
            for await part in group { collected.append(part) }
            return collected
        }
        var result = IngestResult()
        for (_, part) in parts.sorted(by: { $0.0 < $1.0 }) { result.merge(part) }
        return result
    }

    /// 剪贴板 (⌘V、导入菜单的「粘贴」): 图片和 PDF 都当文件表示读, 不解码, 复制的文件引用也认。
    /// 先问有没有相关类型: hasImages 这类查询不会触发系统的「允许粘贴」提示, 没有就根本不去读内容。
    func ingest(pasteboard: UIPasteboard, allowing kinds: Set<HostFileKind>) async -> IngestResult {
        guard Self.hasImportable(pasteboard) else { return IngestResult() }
        return await ingest(providers: pasteboard.itemProviders, allowing: kinds)
    }

    static func hasImportable(_ pasteboard: UIPasteboard) -> Bool {
        pasteboard.hasImages || pasteboard.hasURLs || pasteboard.contains(pasteboardTypes: [UTType.pdf.identifier])
    }

    /// 文件选择器和测试钩子给的 URL: 文件或目录 (递归), 拷贝进沙盒。
    func ingest(urls: [URL], allowing kinds: Set<HostFileKind>) async -> IngestResult {
        await limiter.run { importSync(urls: urls, move: false, allowing: kinds) }
    }

    private func load(_ provider: NSItemProvider, allowing kinds: Set<HostFileKind>) async -> IngestResult {
        let suggested = provider.suggestedName
        if kinds.contains(.pdf), provider.hasItemConformingToTypeIdentifier(UTType.pdf.identifier) {
            return await loadFile(provider, type: .pdf) { [files] url in
                var result = IngestResult()
                result.pdfs = [try files.importPDF(from: url, name: Self.name(for: url, suggested: suggested, type: .pdf), move: true)]
                return result
            }
        }
        if kinds.contains(.image), provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
            return await loadFile(provider, type: .image) { [files] url in
                var result = IngestResult()
                result.images = [try files.adopt(url, name: Self.name(for: url, suggested: suggested, type: .image), move: true)]
                return result
            }
        }
        if provider.hasItemConformingToTypeIdentifier(UTType.folder.identifier) {
            return await loadFolder(provider, allowing: kinds)
        }
        if provider.canLoadObject(ofClass: URL.self) {
            return await loadWebLink(provider, allowing: kinds)
        }
        return IngestResult(ignored: 1)
    }

    /// 只带一个链接的拖放 (从网页拖一个指向图片或 PDF 的链接): 下载到临时文件, 按应答的类型或文件头判断是图片还是 PDF。
    /// 文件和目录的拖放自己带文件表示, 走不到这里; 剪贴板里复制的本地文件引用 (file:) 在这里按文件读, 读不到 (别的 App 的沙盒) 就算忽略。
    private func loadWebLink(_ provider: NSItemProvider, allowing kinds: Set<HostFileKind>) async -> IngestResult {
        let link: URL? = await withCheckedContinuation { continuation in
            _ = provider.loadObject(ofClass: URL.self) { url, _ in continuation.resume(returning: url) }
        }
        guard let link else { return IngestResult(ignored: 1) }
        if link.isFileURL { return importSync(urls: [link], move: false, allowing: kinds) }
        guard link.scheme == "http" || link.scheme == "https" else { return IngestResult(ignored: 1) }
        let downloaded: (URL, URLResponse)
        do {
            downloaded = try await URLSession.shared.download(from: link)
        } catch {
            return IngestResult(ignored: 1)
        }
        let (temporary, response) = downloaded
        defer { try? FileManager.default.removeItem(at: temporary) }
        guard (response as? HTTPURLResponse)?.statusCode == 200,
              let type = Self.webType(mimeType: response.mimeType, header: Self.firstBytes(of: temporary)),
              let kind = Optional(type.conforms(to: .pdf) ? HostFileKind.pdf : .image), kinds.contains(kind)
        else { return IngestResult(ignored: 1) }
        let name = Self.linkFileName(link, type: type)
        var result = IngestResult()
        do {
            switch kind {
            case .pdf: result.pdfs = [try files.importPDF(from: temporary, name: name, move: false)]
            case .image: result.images = [try files.adopt(temporary, name: name, move: false)]
            }
        } catch {
            result.ignored = 1
        }
        return result
    }

    /// 系统给的临时文件在回调返回后就被删掉, 所以必须在回调里搬走。
    private func loadFile(_ provider: NSItemProvider, type: UTType, _ handle: @escaping @Sendable (URL) throws -> IngestResult) async -> IngestResult {
        await withCheckedContinuation { continuation in
            _ = provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, _ in
                guard let url, let result = try? handle(url) else {
                    continuation.resume(returning: IngestResult(ignored: 1))
                    return
                }
                continuation.resume(returning: result)
            }
        }
    }

    /// 目录只能原地读 (带安全作用域), 同样要在回调里拷完。
    private func loadFolder(_ provider: NSItemProvider, allowing kinds: Set<HostFileKind>) async -> IngestResult {
        await withCheckedContinuation { continuation in
            _ = provider.loadInPlaceFileRepresentation(forTypeIdentifier: UTType.folder.identifier) { url, _, _ in
                guard let url else {
                    continuation.resume(returning: IngestResult(ignored: 1))
                    return
                }
                continuation.resume(returning: self.importSync(urls: [url], move: false, allowing: kinds))
            }
        }
    }

    func importSync(urls: [URL], move: Bool, allowing kinds: Set<HostFileKind>) -> IngestResult {
        let scoped = urls.filter { $0.startAccessingSecurityScopedResource() }
        defer { scoped.forEach { $0.stopAccessingSecurityScopedResource() } }
        var result = IngestResult()
        for url in urls {
            if Self.isDirectory(url) {
                for file in Self.files(inDirectory: url, allowing: kinds) { result.merge(importFile(file, move: move, allowing: kinds)) }
            } else {
                result.merge(importFile(url, move: move, allowing: kinds))
            }
        }
        return result
    }

    private func importFile(_ url: URL, move: Bool, allowing kinds: Set<HostFileKind>) -> IngestResult {
        var result = IngestResult()
        guard let kind = HostFileKind(url: url), kinds.contains(kind) else { return IngestResult(ignored: 1) }
        do {
            switch kind {
            case .pdf: result.pdfs = [try files.importPDF(from: url, move: move)]
            case .image: result.images = [try files.adopt(url, move: move)]
            }
        } catch {
            result.ignored = 1
        }
        return result
    }

    // MARK: - 纯函数

    /// 下载下来的东西是什么图片或 PDF: 先信应答的类型, 类型不明 (octet-stream、缺失) 时看文件头。都不是就返回 nil (比如 HTML 页面)。
    /// 返回具体的类型而不只是 图片/PDF, 因为文件名的扩展名和上传时的 MIME 都靠它。
    static func webType(mimeType: String?, header: Data) -> UTType? {
        if let mimeType, let type = UTType(mimeType: mimeType), type.conforms(to: .pdf) || type.conforms(to: .image) {
            return type
        }
        let bytes = [UInt8](header.prefix(12))
        func starts(_ prefix: [UInt8], at offset: Int = 0) -> Bool {
            bytes.count >= offset + prefix.count && Array(bytes[offset..<offset + prefix.count]) == prefix
        }
        if starts(Array("%PDF-".utf8)) { return .pdf }
        if starts([0x89, 0x50, 0x4E, 0x47]) { return .png }
        if starts([0xFF, 0xD8, 0xFF]) { return .jpeg }
        if starts(Array("GIF8".utf8)) { return .gif }
        if starts(Array("BM".utf8)) { return .bmp }
        if starts([0x49, 0x49, 0x2A, 0x00]) || starts([0x4D, 0x4D, 0x00, 0x2A]) { return .tiff }
        if starts(Array("RIFF".utf8)) && starts(Array("WEBP".utf8), at: 8) { return .webP }
        if starts(Array("ftyp".utf8), at: 4) { return .heic }
        return nil
    }

    static func firstBytes(of url: URL) -> Data {
        guard let handle = try? FileHandle(forReadingFrom: url) else { return Data() }
        defer { try? handle.close() }
        return (try? handle.read(upToCount: 12)) ?? Data()
    }

    /// 链接路径的最后一段 (去掉百分号编码); 路径是空的就返回 nil。
    static func linkName(_ link: URL) -> String? {
        let last = link.lastPathComponent
        return last.isEmpty || last == "/" ? nil : last
    }

    /// 下载下来的文件名: 链接最后一段, 扩展名和实际类型对不上 (或没有) 时补上类型的扩展名。临时文件的扩展名是 .tmp, 不能用。
    static func linkFileName(_ link: URL, type: UTType) -> String {
        let base = linkName(link) ?? "download"
        if let current = UTType(filenameExtension: (base as NSString).pathExtension), current.conforms(to: type) || type.conforms(to: current) {
            return base
        }
        guard let ext = type.preferredFilenameExtension else { return base }
        return "\(base).\(ext)"
    }

    static func isDirectory(_ url: URL) -> Bool {
        (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true
    }

    /// 目录里允许的文件, 递归, 按相对路径做自然排序 (page2 在 page10 前), 漫画的页序靠它。
    static func files(inDirectory directory: URL, allowing kinds: Set<HostFileKind>) -> [URL] {
        let keys: [URLResourceKey] = [.isRegularFileKey]
        guard let walker = FileManager.default.enumerator(
            at: directory, includingPropertiesForKeys: keys, options: [.skipsHiddenFiles, .skipsPackageDescendants]
        ) else { return [] }
        var found: [URL] = []
        for case let url as URL in walker {
            guard (try? url.resourceValues(forKeys: Set(keys)))?.isRegularFile == true,
                  let kind = HostFileKind(url: url), kinds.contains(kind) else { continue }
            found.append(url)
        }
        let base = directory.standardizedFileURL.path
        func relative(_ url: URL) -> String {
            let path = url.standardizedFileURL.path
            return path.hasPrefix(base) ? String(path.dropFirst(base.count)) : path
        }
        return found.sorted { relative($0).localizedStandardCompare(relative($1)) == .orderedAscending }
    }

    /// 文件名: 优先 suggestedName (拖放和相册给的是用户看到的名字, 临时文件名有时只是 "PNG图像"), 扩展名取自文件或类型。
    static func name(for url: URL, suggested: String?, type: UTType) -> String {
        guard let suggested, !suggested.isEmpty else { return url.lastPathComponent }
        let ext = url.pathExtension.isEmpty ? (type.preferredFilenameExtension ?? "") : url.pathExtension
        if ext.isEmpty || (suggested as NSString).pathExtension.caseInsensitiveCompare(ext) == .orderedSame { return suggested }
        return "\(suggested).\(ext)"
    }
}
