import Foundation
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

/// NSItemProvider 没有标 Sendable, 但读取它的回调本来就是跨线程设计的。
struct ProviderBox: @unchecked Sendable {
    let value: NSItemProvider
    init(_ value: NSItemProvider) { self.value = value }
}

/// 把拖放 / 选择得到的东西变成沙盒里的文件。
/// 每个 NSItemProvider 只读一种表示 (PDF 或图片, 不再试第二种), 并发读取不超过 limit 个。
final class FileIngestor: @unchecked Sendable {
    static let concurrency = 4

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
        return IngestResult(ignored: 1)
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
