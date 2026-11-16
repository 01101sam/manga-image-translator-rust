import NativeShell
import SwiftUI
import UIKit
import UniformTypeIdentifiers

extension ManagedFiles {
    /// 整个进程共用一份: 多个窗口里的 id 互通, 启动清理也只做一次。
    static let shared: ManagedFiles = {
        let files = ManagedFiles.standard()
        files.cleanOnLaunch()
        return files
    }()
}

/// 薄宿主: 把 client-ui 的 TSX bundle 放进 NshAppView, 并实现 HOST_PROTOCOL.md 里的原生调用。
/// 业务界面全在 TSX 里, 这里只有 Bonjour、Keychain、拖放与选择器、流式上传、PDFKit。
@MainActor
final class NshHostViewController: UIViewController, UIDropInteractionDelegate {
    private let hooks: TestHooks
    private let files = ManagedFiles.shared
    private let ingestor: FileIngestor
    private let uploads: UploadClient
    private let session: HostSession
    private let renderLimiter = AsyncLimiter(limit: 2)
    private lazy var pickers = PickerPresenter(host: self)
    private var appView: NshAppView?
    private var errorOverlay: UIStackView?
    private var autoHooksScheduled = false
    private var debugDragSource: DebugDragSource?

    init(hooks: TestHooks) {
        self.hooks = hooks
        self.ingestor = FileIngestor(files: ManagedFiles.shared)
        self.uploads = UploadClient(files: ManagedFiles.shared)
        let tokens: TokenStoring = hooks.isolatesSession ? MemoryTokenStore(value: hooks.paired) : KeychainTokenStore()
        if hooks.forgetPairing { tokens.clear() }
        self.session = HostSession(tokens: tokens)
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) {
        fatalError("NshHostViewController is created in code")
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.addInteraction(UIDropInteraction(delegate: self))
        session.onDaemonsChanged = { [weak self] daemons in self?.appView?.send("session.daemons", daemons) }
        session.start(seed: hooks.browseDaemon)
        mountApp()
        if let folder = hooks.debugDragSource {
            let source = DebugDragSource(folder: URL(fileURLWithPath: folder), count: 3, scratch: files.scratchFile(named: "debug-drag"))
            source.install(in: view)
            debugDragSource = source
        }
    }

    // MARK: - NSH

    /// 每次重新载入都换一个新的 NshAppView: JS 虚拟机出错停止后, 复用旧的视图没有保证。
    private func mountApp() {
        errorOverlay?.removeFromSuperview()
        errorOverlay = nil
        appView?.removeFromSuperview()
        let app = NshAppView()
        app.translatesAutoresizingMaskIntoConstraints = false
        app.onError = { [weak self] message in
            NSLog("NSH_ERROR %@", message)
            self?.showError(message)
        }
        // 图片缓存只占磁盘, 上限定死, 大量大图不会把设备存储吃满。
        app.cacheOptions = NshCacheOptions(location: .system, images: .maxSize(megabytes: 256))
        view.addSubview(app)
        NSLayoutConstraint.activate([
            app.topAnchor.constraint(equalTo: view.topAnchor),
            app.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            app.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            app.trailingAnchor.constraint(equalTo: view.trailingAnchor),
        ])
        appView = app
        registerHandlers(on: app)
        let files = files
        app.register(component: "it.pdf") { context in PdfComponent(context: context, files: files) }
        if let section = hooks.section { app.params = ["section": section] }
        let bundled = Bundle.main.url(forResource: "bundle", withExtension: "nshc", subdirectory: "nsh")
        guard let bundle = hooks.bundlePath.map({ URL(fileURLWithPath: $0) }) ?? bundled else {
            showError("App 里没有界面 bundle (nsh/bundle.nshc)。先运行 scripts/build-ui.sh native 再构建。")
            return
        }
        do {
            try app.loadBundle(at: bundle)
        } catch {
            showError("界面 bundle 载入失败: \(error.localizedDescription)")
        }
    }

    private func showError(_ message: String) {
        guard errorOverlay == nil else { return }
        let label = UILabel()
        label.text = message
        label.numberOfLines = 0
        label.textAlignment = .center
        label.accessibilityIdentifier = "host-error"
        var configuration = UIButton.Configuration.filled()
        configuration.title = "重新载入"
        let button = UIButton(configuration: configuration, primaryAction: UIAction { [weak self] _ in self?.mountApp() })
        button.accessibilityIdentifier = "host-reload"
        let stack = UIStackView(arrangedSubviews: [label, button])
        stack.axis = .vertical
        stack.spacing = 16
        stack.alignment = .center
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            stack.leadingAnchor.constraint(greaterThanOrEqualTo: view.leadingAnchor, constant: 24),
            stack.trailingAnchor.constraint(lessThanOrEqualTo: view.trailingAnchor, constant: -24),
        ])
        errorOverlay = stack
    }

    // MARK: - 宿主调用

    private func registerHandlers(on app: NshAppView) {
        app.handle("session.snapshot") { [weak self] _ -> Any? in
            guard let self else { return NSNull() }
            scheduleAutoHooks()
            var snapshot = session.snapshot()
            snapshot["imports"] = Self.imports
            return snapshot
        }
        app.handle("session.saveToken") { [weak self] data -> Any? in
            try self?.session.saveToken(data)
            return NSNull()
        }
        app.handle("session.clearToken") { [weak self] _ -> Any? in
            self?.session.clearToken()
            return NSNull()
        }
        app.handle("files.pick") { [weak self] data -> Any? in
            guard let self else { return ["files": [Any]()] }
            let args = data as? [String: Any]
            let limit = hostInt(args?["limit"]) ?? 200
            return ["files": try await pickFiles(limit: limit, source: (args?["source"] as? String) ?? "files").map(\.json)]
        }
        app.handle("files.release") { [weak self] data, _ in
            let ids = ((data as? [String: Any])?["ids"] as? [Any])?.compactMap { $0 as? String } ?? []
            self?.files.release(ids)
        }
        app.handle("upload.start") { [weak self] data -> Any? in
            guard let self else { return UploadOutcome.failed(.network, message: "界面已关闭").json }
            return await startUpload(data).json
        }
        app.handle("artifact.save") { [weak self] data -> Any? in
            guard let self, let args = data as? [String: Any], let text = args["url"] as? String, let url = URL(string: text)
            else { throw HostCallError("保存参数不完整") }
            let file = try await ArtifactSaver.download(from: url, filename: (args["filename"] as? String) ?? "image", files: files)
            ArtifactSaver.present(file, from: self)
            return NSNull()
        }
        app.handle("pdf.pick") { [weak self] _ -> Any? in
            guard let self else { return ["file": NSNull()] }
            let file = try await pickPDF()
            return ["file": file.map { $0.json as Any } ?? NSNull()]
        }
        app.handle("pdf.open") { [weak self] data -> Any? in
            let id = (data as? [String: Any])?["fileId"] as? String
            return ["file": id.flatMap { self?.files.pdfRef(for: $0) }.map { $0.json as Any } ?? NSNull()]
        }
        app.handle("pdf.export") { [weak self] data -> Any? in
            guard let self else { throw HostCallError("界面已关闭") }
            let outcome = try await exportPDF(data)
            return ["translated": outcome.translated, "missing": outcome.missing]
        }
        app.handle("pdf.release") { [weak self] data, _ in
            if let id = (data as? [String: Any])?["fileId"] as? String { self?.files.releasePDF(id) }
        }
        app.handle("pdf.renderPage") { [weak self] data -> Any? in
            guard let self else { throw HostCallError("界面已关闭") }
            return ["fileRef": try await renderPage(data).json]
        }
    }

    private func startUpload(_ data: Any?) async -> UploadOutcome {
        guard let args = data as? [String: Any], let id = args["fileId"] as? String,
              let text = args["url"] as? String, let endpoint = URL(string: text), let token = args["token"] as? String
        else { return .failed(.rejected, message: "上传参数不完整") }
        guard let file = files.url(for: id) else { return .failed(.rejected, message: "文件已不可用, 请重新添加") }
        let overrides = args["overrides"] ?? [String: Any]()
        guard JSONSerialization.isValidJSONObject(overrides),
              let encoded = try? JSONSerialization.data(withJSONObject: overrides, options: [.sortedKeys])
        else { return .failed(.rejected, message: "覆盖参数不是合法 JSON") }
        let outcome = await uploads.upload(UploadRequest(
            file: file, filename: file.lastPathComponent, mime: ManagedFiles.mimeType(of: file),
            endpoint: endpoint, token: token, overrides: String(decoding: encoded, as: UTF8.self)
        ))
        if case .accepted = outcome { files.release([id]) }
        return outcome
    }

    // MARK: - 选择

    /// 宿主声明的导入入口, TSX 的导入菜单只显示这些 (HOST_PROTOCOL.md 的 session.snapshot)。
    private static let imports = ["photos", "files", "pdf"]

    /// photos 只出图片; files 里图片、PDF 和文件夹都行 (文件夹递归, 按自然序)。选出来的 PDF 不放进返回值, 走 pdf.added,
    /// 与拖放一样由 TSX 打开阅读器。
    private func pickFiles(limit: Int, source: String) async throws -> [FileRef] {
        let outcome: PickerOutcome
        if let folder = hooks.pickFolder {
            outcome = .urls([URL(fileURLWithPath: folder)])
        } else {
            guard !pickers.isBusy else { throw HostCallError("已经有一个选择器打开了") }
            switch source {
            case "photos": outcome = await pickers.pickPhotos(limit: limit)
            case "files": outcome = await pickers.pickDocuments(types: [.image, .pdf, .folder], multiple: true)
            default: throw HostCallError("不支持的导入来源: \(source)")
            }
        }
        let result = await ingest(outcome, allowing: [.image, .pdf])
        let delivery = ImportDelivery(result)
        send(delivery)
        return trim(result.images, to: limit)
    }

    private func pickPDF() async throws -> FileRef? {
        if let path = hooks.pickPDF {
            return await ingestor.ingest(urls: [URL(fileURLWithPath: path)], allowing: [.pdf]).pdfs.first
        }
        guard !pickers.isBusy else { throw HostCallError("已经有一个选择器打开了") }
        let result = await ingest(await pickers.pickDocuments(types: [.pdf], multiple: false), allowing: [.pdf])
        return result.pdfs.first
    }

    private func ingest(_ outcome: PickerOutcome, allowing kinds: Set<HostFileKind>) async -> IngestResult {
        switch outcome {
        case .providers(let providers): return await ingestor.ingest(providers: providers, allowing: kinds)
        case .urls(let urls): return await ingestor.ingest(urls: urls, allowing: kinds)
        case .cancelled: return IngestResult()
        }
    }

    /// 超出 limit 的文件已经拷进沙盒了, 立刻释放, 不留孤儿。
    private func trim(_ images: [FileRef], to limit: Int) -> [FileRef] {
        guard limit > 0, images.count > limit else { return images }
        files.release(images[limit...].map(\.id))
        return Array(images[..<limit])
    }

    /// 发 PDF 和提示; 图片走返回值 (files.pick) 或 deliver (其余入口)。多出来的 PDF 在这里放掉。
    private func send(_ delivery: ImportDelivery, includeImages: Bool = false) {
        delivery.droppedPDFs.forEach { files.releasePDF($0.id) }
        for event in delivery.events(includeImages: includeImages) { appView?.send(event.name, event.payload) }
    }

    // MARK: - PDF

    private func renderPage(_ data: Any?) async throws -> FileRef {
        guard let args = data as? [String: Any], let id = args["fileId"] as? String, let page = hostInt(args["page"]), page >= 0
        else { throw HostCallError("渲染参数不完整") }
        guard let pdf = files.pdfURL(for: id) else { throw HostCallError("这个 PDF 已经不在了, 请重新打开") }
        let longEdge = PdfRenderer.clampedLongEdge(hostInt(args["longEdge"]))
        let base = (files.pdfDisplayName(for: id) as NSString).deletingPathExtension
        let name = "\(base)-p\(String(format: "%03d", page + 1)).png"
        let files = files
        // 渲染要整页位图, 同时最多两页, 避免连点翻译时内存叠加。
        let outcome: Result<FileRef, HostCallError> = await renderLimiter.run {
            await Task.detached(priority: .userInitiated) {
                let scratch = files.scratchFile(named: name)
                defer { try? FileManager.default.removeItem(at: scratch) }
                do {
                    try PdfRenderer.render(document: pdf, page: page, longEdge: longEdge, to: scratch)
                    return .success(try files.adopt(scratch, name: name, move: true))
                } catch let error as HostCallError {
                    return .failure(error)
                } catch {
                    return .failure(HostCallError("渲染失败: \(error.localizedDescription)"))
                }
            }.value
        }
        return try outcome.get()
    }

    /// 译文页换进原 PDF 拼成新 PDF, 逐页下载逐页写 (PdfExporter), 写完弹系统分享面板。
    private func exportPDF(_ data: Any?) async throws -> PdfExporter.Outcome {
        guard let args = data as? [String: Any], let id = args["fileId"] as? String, let list = args["pages"] as? [Any]
        else { throw HostCallError("导出参数不完整") }
        guard let source = files.pdfURL(for: id) else { throw HostCallError("这个 PDF 已经不在了, 请重新打开") }
        var remote: [Int: URL] = [:]
        for case let entry as [String: Any] in list {
            guard let page = hostInt(entry["page"]), page >= 0, let text = entry["artifactUrl"] as? String, let url = URL(string: text)
            else { throw HostCallError("导出参数不完整") }
            remote[page] = url
        }
        let base = (files.pdfDisplayName(for: id) as NSString).deletingPathExtension
        let output = files.sharedFile(named: "\(base)-translated.pdf")
        let files = files
        let outcome: PdfExporter.Outcome
        do {
            outcome = try await Task.detached(priority: .utility) {
                try await PdfExporter.compose(
                    source: source, output: output,
                    artifact: { index in
                        guard let url = remote[index] else { return nil }
                        return try await ArtifactSaver.download(from: url, filename: "page", files: files)
                    },
                    release: { try? FileManager.default.removeItem(at: $0.deletingLastPathComponent()) }
                )
            }.value
        } catch {
            try? FileManager.default.removeItem(at: output.deletingLastPathComponent())
            throw error
        }
        ArtifactSaver.present(output, from: self)
        return outcome
    }

    // MARK: - 拖放

    func dropInteraction(_ interaction: UIDropInteraction, canHandle session: UIDropSession) -> Bool {
        session.hasItemsConforming(toTypeIdentifiers: [
            UTType.image.identifier, UTType.pdf.identifier, UTType.folder.identifier, UTType.url.identifier,
        ])
    }

    func dropInteraction(_ interaction: UIDropInteraction, sessionDidUpdate session: UIDropSession) -> UIDropProposal {
        UIDropProposal(operation: .copy)
    }

    func dropInteraction(_ interaction: UIDropInteraction, sessionDidEnter session: UIDropSession) {
        appView?.send("files.dragState", ["active": true])
    }

    func dropInteraction(_ interaction: UIDropInteraction, sessionDidExit session: UIDropSession) {
        appView?.send("files.dragState", ["active": false])
    }

    func dropInteraction(_ interaction: UIDropInteraction, sessionDidEnd session: UIDropSession) {
        appView?.send("files.dragState", ["active": false])
    }

    func dropInteraction(_ interaction: UIDropInteraction, performDrop session: UIDropSession) {
        let providers = session.items.map(\.itemProvider)
        Task { [ingestor] in
            deliver(await ingestor.ingest(providers: providers, allowing: [.image, .pdf]))
        }
    }

    private func deliver(_ result: IngestResult) {
        send(ImportDelivery(result), includeImages: true)
    }

    // MARK: - 测试钩子

    /// -auto-import-folder: TSX 起来后模拟一次拖放。文件先克隆一份, 再包成 NSItemProvider 走与真实拖放相同的读取路径,
    /// 因为拖放的读取会把文件搬走, 不能动原目录。
    private func scheduleAutoHooks() {
        guard !autoHooksScheduled, let folder = hooks.autoImportFolder else { return }
        autoHooksScheduled = true
        let viaURLs = hooks.autoImportViaURLs
        Task {
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            HostMetrics.log("hook start")
            if viaURLs {
                let result = await HostMetrics.measured("ingest urls") { await ingestor.ingest(urls: [URL(fileURLWithPath: folder)], allowing: [.image, .pdf]) }
                HostMetrics.log("ingested \(result.images.count) via urls")
                deliver(result)
                return
            }
            let clones = Self.cloneFiles(inFolder: URL(fileURLWithPath: folder), into: files.scratchFile(named: "drop-hook"))
            HostMetrics.log("cloned \(clones.count)")
            let providers = clones.compactMap(DebugProvider.make)
            HostMetrics.log("providers created \(providers.count)")
            let result = await HostMetrics.measured("ingest providers") { await ingestor.ingest(providers: providers, allowing: [.image, .pdf]) }
            HostMetrics.log("ingested \(result.images.count) via providers")
            deliver(result)
        }
    }

    private static func cloneFiles(inFolder folder: URL, into directory: URL) -> [URL] {
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return FileIngestor.files(inDirectory: folder, allowing: [.image, .pdf]).compactMap { source in
            let target = directory.appendingPathComponent(source.lastPathComponent)
            return (try? FileManager.default.copyItem(at: source, to: target)) == nil ? nil : target
        }
    }
}

/// 测试钩子用: 进程常驻内存, 方便把 RSS 的涨幅对到具体步骤上。
enum HostMetrics {
    static func residentMB() -> Int {
        var info = mach_task_basic_info()
        var count = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info>.size / MemoryLayout<natural_t>.size)
        let status = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &count)
            }
        }
        return status == KERN_SUCCESS ? Int(info.resident_size / 1_048_576) : -1
    }

    /// phys_footprint 是系统判断内存压力和 jetsam 的口径; 干净的文件映射页算进 RSS 但不算它。
    static func footprintMB() -> Int {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
        let status = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return status == KERN_SUCCESS ? Int(info.phys_footprint / 1_048_576) : -1
    }

    static func log(_ stage: String) {
        NSLog("HOSTHOOK %@ rss=%dMB footprint=%dMB", stage, residentMB(), footprintMB())
    }

    /// 跑 body 期间每 20 ms 采一次样, 取 RSS 和 footprint 的峰值: 平均值和结束时的数看不出摄入过程中的尖峰。
    static func measure<T>(_ body: () async -> T) async -> (value: T, peak: (rss: Int, footprint: Int)) {
        let sampler = PeakSampler()
        sampler.start()
        let value = await body()
        return (value, sampler.stop())
    }

    static func measured<T>(_ stage: String, _ body: () async -> T) async -> T {
        let (value, peak) = await measure(body)
        NSLog("HOSTHOOK %@ peak rss=%dMB footprint=%dMB, after rss=%dMB footprint=%dMB", stage, peak.rss, peak.footprint, residentMB(), footprintMB())
        return value
    }
}

private final class PeakSampler: @unchecked Sendable {
    private let queue = DispatchQueue(label: "peak-sampler")
    private var timer: DispatchSourceTimer?
    private var rss = 0
    private var footprint = 0

    func start() {
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now(), repeating: .milliseconds(20))
        timer.setEventHandler { [self] in
            rss = max(rss, HostMetrics.residentMB())
            footprint = max(footprint, HostMetrics.footprintMB())
        }
        timer.resume()
        self.timer = timer
    }

    func stop() -> (rss: Int, footprint: Int) {
        timer?.cancel()
        return queue.sync { (rss, footprint) }
    }
}

struct NshHostView: UIViewControllerRepresentable {
    let hooks: TestHooks

    func makeUIViewController(context: Context) -> NshHostViewController {
        NshHostViewController(hooks: hooks)
    }

    func updateUIViewController(_ uiViewController: NshHostViewController, context: Context) {}
}
