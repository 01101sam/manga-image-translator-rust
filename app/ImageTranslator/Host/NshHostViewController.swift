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
            return session.snapshot()
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
            let limit = hostInt((data as? [String: Any])?["limit"]) ?? 200
            return ["files": try await pickImages(limit: limit).map(\.json)]
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

    private func pickImages(limit: Int) async throws -> [FileRef] {
        if let folder = hooks.pickFolder {
            return take(limit, from: await ingestor.ingest(urls: [URL(fileURLWithPath: folder)], allowing: [.image]))
        }
        guard !pickers.isBusy else { throw HostCallError("已经有一个选择器打开了") }
        guard let source = await pickers.chooseImageSource() else { return [] }
        let outcome: PickerOutcome
        switch source {
        case .photos: outcome = await pickers.pickPhotos(limit: limit)
        case .files: outcome = await pickers.pickDocuments(types: [.image], multiple: true)
        case .folder: outcome = await pickers.pickDocuments(types: [.folder], multiple: false)
        }
        return take(limit, from: await ingest(outcome, allowing: [.image]))
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
    private func take(_ limit: Int, from result: IngestResult) -> [FileRef] {
        notifyIgnored(result.ignored)
        guard limit > 0, result.images.count > limit else { return result.images }
        files.release(result.images[limit...].map(\.id))
        return Array(result.images[..<limit])
    }

    private func notifyIgnored(_ count: Int) {
        if count > 0 { appView?.send("host.notice", ["text": "已忽略 \(count) 个不是图片的文件"]) }
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

    // MARK: - 拖放

    func dropInteraction(_ interaction: UIDropInteraction, canHandle session: UIDropSession) -> Bool {
        session.hasItemsConforming(toTypeIdentifiers: [
            UTType.image.identifier, UTType.pdf.identifier, UTType.folder.identifier, UTType.fileURL.identifier,
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

    /// 图片走 files.added, PDF 走 pdf.added, 其余给一条提示。
    private func deliver(_ result: IngestResult) {
        if !result.images.isEmpty { appView?.send("files.added", result.images.map(\.json)) }
        if !result.pdfs.isEmpty { appView?.send("pdf.added", result.pdfs.map(\.json)) }
        notifyIgnored(result.ignored)
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
                let result = await ingestor.ingest(urls: [URL(fileURLWithPath: folder)], allowing: [.image, .pdf])
                HostMetrics.log("ingested \(result.images.count) via urls")
                deliver(result)
                return
            }
            let clones = Self.cloneFiles(inFolder: URL(fileURLWithPath: folder), into: files.scratchFile(named: "drop-hook"))
            HostMetrics.log("cloned \(clones.count)")
            let providers = clones.compactMap(DebugProvider.make)
            HostMetrics.log("providers created \(providers.count)")
            let result = await ingestor.ingest(providers: providers, allowing: [.image, .pdf])
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

    static func log(_ stage: String) {
        NSLog("HOSTHOOK %@ rss=%dMB", stage, residentMB())
    }
}

struct NshHostView: UIViewControllerRepresentable {
    let hooks: TestHooks

    func makeUIViewController(context: Context) -> NshHostViewController {
        NshHostViewController(hooks: hooks)
    }

    func updateUIViewController(_ uiViewController: NshHostViewController, context: Context) {}
}
