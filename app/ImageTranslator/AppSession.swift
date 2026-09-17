import Combine
import Foundation
import UIKit
import UniformTypeIdentifiers

enum AppTab: Hashable {
    case jobs
    case reader
    case config
}

@MainActor
final class AppSession: ObservableObject {
    @Published private(set) var pairing: PairingPhase
    @Published var jobs: [JobRow] = []
    @Published var artifacts: [String: Data] = [:]
    @Published var engine: EngineState?
    @Published var banner: String?
    @Published var pairingCode: String = ""
    @Published var selectedTab: AppTab = .jobs
    @Published var pendingPDF: URL?

    let tokens: TokenStoring
    let browser: BonjourBrowser
    private let session: URLSession
    private var pollTask: Task<Void, Never>?
    private var browserBag = Set<AnyCancellable>()
    private var pendingImagePath: String?
    private var pendingFolderPath: String?

    init(tokens: TokenStoring? = nil, session: URLSession = .shared, hooks: TestHooks = .fromProcessInfo()) {
        let resolvedTokens: TokenStoring
        if let tokens {
            resolvedTokens = tokens
        } else if hooks.isolatesSession {
            resolvedTokens = MemoryTokenStore(value: hooks.paired)
        } else {
            resolvedTokens = KeychainTokenStore()
        }
        self.tokens = resolvedTokens
        self.session = session
        let browser = BonjourBrowser()
        self.browser = browser
        if let paired = hooks.paired ?? resolvedTokens.load() {
            pairing = .paired(paired.endpoint, paired.token)
        } else {
            pairing = .browsing
        }
        if hooks.openPDF != nil {
            selectedTab = .reader
        }
        pendingPDF = hooks.openPDF.flatMap { hooks.resolveFile($0) }
        pendingImagePath = hooks.autoSubmitImage
        pendingFolderPath = hooks.autoImportFolder
        browser.objectWillChange
            .sink { [weak self] _ in
                self?.objectWillChange.send()
            }
            .store(in: &browserBag)
        if let seeded = hooks.browseDaemon {
            browser.seed(seeded)
        } else if hooks.paired == nil {
            browser.start()
        }
        startPolling()
        if pendingImagePath != nil || pendingFolderPath != nil {
            Task {
                for _ in 0..<8 {
                    try? await Task.sleep(nanoseconds: 250_000_000)
                    if pendingImagePath == nil, pendingFolderPath == nil { break }
                    consumePendingHooks()
                }
            }
        }
    }

    func consumePendingPDF() -> URL? {
        let url = pendingPDF
        pendingPDF = nil
        return url
    }

    func openPDF(_ url: URL) {
        pendingPDF = url
        selectedTab = .reader
    }

    func ingestDrop(_ providers: [NSItemProvider]) -> Bool {
        guard !providers.isEmpty else { return false }
        let batch = DropBatch(session: self, remaining: providers.count)
        for provider in providers {
            startDropLoad(provider, batch: batch)
        }
        return true
    }

    func consumePendingHooks() {
        consumePendingImageIfNeeded()
        consumePendingFolderIfNeeded()
    }

    func consumePendingImageIfNeeded() {
        guard let raw = pendingImagePath else { return }
        let hooks = TestHooks.fromProcessInfo()
        let paths = raw.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
        var items: [(Data, String, String)] = []
        for path in paths {
            let url = hooks.resolveFile(path)
                ?? Bundle.main.url(forResource: "sample", withExtension: "jpg")
            guard let url, let data = try? Data(contentsOf: url), !data.isEmpty else {
                banner = "测试图无法读取"
                return
            }
            items.append((data, url.lastPathComponent, mimeFor(url)))
        }
        pendingImagePath = nil
        Task {
            for item in items {
                await submitImage(item.0, filename: item.1, mime: item.2)
            }
        }
    }

    func consumePendingFolderIfNeeded() {
        guard let path = pendingFolderPath else { return }
        guard let url = TestHooks.fromProcessInfo().resolveFolder(path) else {
            banner = "测试文件夹无法读取"
            return
        }
        pendingFolderPath = nil
        Task {
            await importURLs([url])
        }
    }

    var client: DaemonClient? {
        guard let endpoint = pairing.endpoint else { return nil }
        return DaemonClient(session: session, endpoint: endpoint, token: pairing.token)
    }

    var isPaired: Bool {
        if case .paired = pairing { return true }
        return false
    }

    func select(_ endpoint: DaemonEndpoint) {
        applyPairing(.select(endpoint))
    }

    func submitPairingCode() {
        applyPairing(.submitCode(pairingCode))
    }

    func retryPairing() {
        pairingCode = ""
        applyPairing(.retry)
    }

    func forgetPairing() {
        pairingCode = ""
        applyPairing(.forget)
    }

    @discardableResult
    func submitImage(_ data: Data, filename: String, mime: String = "image/jpeg") async -> Bool {
        banner = nil
        guard var api = client, api.token != nil else { return false }
        do {
            let jobId = try await api.submitJob(image: data, filename: filename, mime: mime, overrides: nil)
            let row = JobRow(
                snapshot: JobSnapshot(jobId: jobId, state: .queued, positionInQueue: nil, error: nil),
                cancelPending: false,
                thumbnail: makeJobThumbnail(data)
            )
            jobs.insert(row, at: 0)
            return true
        } catch DaemonError.unauthorized {
            applyPairing(.unauthorized)
            return false
        } catch {
            await refreshEngineHint(fallback: error.localizedDescription)
            return false
        }
    }

    func removeJobs(at offsets: IndexSet) {
        removeLocalJobs(jobs: &jobs, artifacts: &artifacts, at: offsets)
    }

    func moveJobs(from source: IndexSet, to destination: Int) {
        moveLocalJobs(jobs: &jobs, from: source, to: destination)
    }

    func importURLs(_ urls: [URL]) async {
        for url in urls {
            let accessed = url.startAccessingSecurityScopedResource()
            defer {
                if accessed { url.stopAccessingSecurityScopedResource() }
            }
            var isDir: ObjCBool = false
            if FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir), isDir.boolValue {
                await importFolder(url)
                continue
            }
            if routeImportedURL(url) == .openPDF {
                openPDF(url)
                continue
            }
            guard let data = try? Data(contentsOf: url) else { continue }
            await submitImage(data, filename: url.lastPathComponent, mime: mimeFor(url))
        }
    }

    func requestCancel(_ jobId: String) async {
        guard let idx = jobs.firstIndex(where: { $0.snapshot.jobId == jobId }) else { return }
        let (next, command) = reduceJob(jobs[idx], .requestCancel)
        jobs[idx] = next
        guard case .cancel(let id) = command, var api = client else { return }
        do {
            _ = try await api.cancel(id: id)
            applyJob(id, .cancelAccepted)
        } catch DaemonError.unauthorized {
            applyPairing(.unauthorized)
        } catch {
            applyJob(id, .cancelRejected)
            banner = error.localizedDescription
        }
    }

    func applyPageJob(_ jobId: String, _ life: JobLifecycle) {
        applyJob(jobId, .snapshot(JobSnapshot(jobId: jobId, state: life, positionInQueue: nil, error: nil)))
    }

    private func importFolder(_ root: URL) async {
        let keys: [URLResourceKey] = [.isRegularFileKey, .contentTypeKey]
        let files = imageFiles(in: root, keys: keys)
        for file in files {
            let type = (try? file.resourceValues(forKeys: [.contentTypeKey]))?.contentType
            let mime = type?.preferredMIMEType ?? "image/jpeg"
            if let data = try? Data(contentsOf: file) {
                await submitImage(data, filename: file.lastPathComponent, mime: mime)
            }
        }
    }

    private func applyPairing(_ event: PairingEvent) {
        let (next, command) = reducePairing(pairing, event)
        pairing = next
        switch command {
        case .none:
            break
        case .requestPair:
            Task { await runPairRequest() }
        case .confirm(_, let code):
            Task { await runPairConfirm(code) }
        case .persist(let paired):
            tokens.save(paired)
            banner = nil
        case .clearToken:
            tokens.clear()
            jobs = []
            artifacts = [:]
        }
    }

    private func runPairRequest() async {
        guard var api = client else { return }
        api.token = nil
        do {
            try await api.requestPair()
            applyPairing(.requestSucceeded)
        } catch {
            applyPairing(.requestFailed(error.localizedDescription))
        }
    }

    private func runPairConfirm(_ code: String) async {
        guard var api = client else { return }
        api.token = nil
        do {
            let token = try await api.confirm(code: code)
            applyPairing(.confirmSucceeded(token))
        } catch DaemonError.unauthorized {
            applyPairing(.confirmFailed("配对码错误或已过期"))
        } catch {
            applyPairing(.confirmFailed(error.localizedDescription))
        }
    }

    private func startPolling() {
        pollTask?.cancel()
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.tick()
                try? await Task.sleep(nanoseconds: 1_000_000_000)
            }
        }
    }

    private func tick() async {
        guard isPaired, var api = client else { return }
        let active = jobs.filter { $0.snapshot.state.isActive }
        for row in active {
            do {
                let snap = try await api.job(id: row.snapshot.jobId)
                applyJob(snap.jobId, .snapshot(snap))
                if snap.state == .done {
                    await fetchArtifact(id: snap.jobId, api: api)
                }
                if snap.state == .failed {
                    await refreshEngineHint(fallback: snap.error ?? "Job 失败")
                }
            } catch DaemonError.unauthorized {
                applyPairing(.unauthorized)
                return
            } catch {
                break
            }
        }
    }

    private func fetchArtifact(id: String, api: DaemonClient) async {
        if artifacts[id] != nil { return }
        do {
            let (data, _) = try await api.artifact(id: id)
            artifacts[id] = data
        } catch DaemonError.unauthorized {
            applyPairing(.unauthorized)
        } catch {
            banner = error.localizedDescription
        }
    }

    private func applyJob(_ jobId: String, _ event: JobEvent) {
        guard let idx = jobs.firstIndex(where: { $0.snapshot.jobId == jobId }) else { return }
        let (next, _) = reduceJob(jobs[idx], event)
        jobs[idx] = next
    }

    private func refreshEngineHint(fallback: String) async {
        guard let api = client else {
            banner = fallback
            return
        }
        do {
            let state = try await api.engineStatus()
            engine = state
            if state == .stopped {
                banner = "Engine 已停止，请到配置页启动后再提交。"
            } else {
                banner = fallback
            }
        } catch DaemonError.unauthorized {
            applyPairing(.unauthorized)
        } catch {
            banner = fallback
        }
    }

    private func startDropLoad(_ provider: NSItemProvider, batch: DropBatch) {
        let box = ProviderDropBox(types: provider.registeredTypeIdentifiers, session: self, batch: batch)
        let imageIds = dropImageTypeIdentifiers(for: provider)
        let tryImage = !imageIds.isEmpty || provider.canLoadObject(ofClass: UIImage.self)
        let tryPDF = provider.hasItemConformingToTypeIdentifier(UTType.pdf.identifier)
        let tryFile = provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier)
        let tryURL = provider.hasItemConformingToTypeIdentifier(UTType.url.identifier)
            || provider.canLoadObject(ofClass: NSURL.self)
        let hail = !tryImage && !tryPDF && !tryFile && !tryURL

        // All loads start inside the drop closure (Apple requirement), then resolve by priority.
        if tryImage || tryPDF || hail {
            box.addPending()
            _ = provider.loadTransferable(type: Data.self) { result in
                if case .success(let data) = result, !data.isEmpty {
                    box.setBytes(data)
                }
                box.done()
            }
        }
        if tryImage || hail {
            for id in imageIds {
                box.addPending()
                _ = provider.loadDataRepresentation(forTypeIdentifier: id) { data, _ in
                    if let data, !data.isEmpty {
                        box.setBytes(data)
                    }
                    box.done()
                }
                box.addPending()
                _ = provider.loadFileRepresentation(forTypeIdentifier: id) { url, _ in
                    if let url, let data = try? Data(contentsOf: url), !data.isEmpty {
                        box.setBytes(data)
                    }
                    box.done()
                }
            }
            if provider.canLoadObject(ofClass: UIImage.self) {
                box.addPending()
                _ = provider.loadObject(ofClass: UIImage.self) { object, _ in
                    if let image = object as? UIImage, let data = image.jpegData(compressionQuality: 0.92) {
                        box.setBytes(data)
                    }
                    box.done()
                }
            }
        }
        if tryPDF {
            box.addPending()
            _ = provider.loadDataRepresentation(forTypeIdentifier: UTType.pdf.identifier) { data, _ in
                if let data, !data.isEmpty {
                    box.setPDF(data)
                }
                box.done()
            }
        }
        if tryFile || hail {
            box.addPending()
            _ = provider.loadFileRepresentation(forTypeIdentifier: UTType.fileURL.identifier) { url, _ in
                if let url {
                    box.setFile(copyDroppedFile(url))
                }
                box.done()
            }
            box.addPending()
            provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier, options: nil) { item, _ in
                if let url = dropAnyURL(from: item), url.isFileURL {
                    let accessed = url.startAccessingSecurityScopedResource()
                    box.setFile(copyDroppedFile(url))
                    if accessed { url.stopAccessingSecurityScopedResource() }
                }
                box.done()
            }
        }
        if tryURL || hail {
            box.addPending()
            provider.loadItem(forTypeIdentifier: UTType.url.identifier, options: nil) { item, _ in
                if let url = dropAnyURL(from: item) {
                    if url.isFileURL {
                        let accessed = url.startAccessingSecurityScopedResource()
                        box.setFile(copyDroppedFile(url))
                        if accessed { url.stopAccessingSecurityScopedResource() }
                    } else if isHTTPURL(url) {
                        box.setWeb(url)
                    }
                }
                box.done()
            }
            if provider.canLoadObject(ofClass: NSURL.self) {
                box.addPending()
                _ = provider.loadObject(ofClass: NSURL.self) { object, _ in
                    if let url = object as? URL {
                        if url.isFileURL {
                            box.setFile(copyDroppedFile(url))
                        } else if isHTTPURL(url) {
                            box.setWeb(url)
                        }
                    }
                    box.done()
                }
            }
        }

        if box.pendingCount == 0 {
            batch.finish(images: 0, openedPDF: false, failTypes: provider.registeredTypeIdentifiers)
        }
    }

    fileprivate func resolveDrop(
        bytes: Data?,
        pdf: Data?,
        file: URL?,
        web: URL?,
        types: [String],
        batch: DropBatch
    ) async {
        if let bytes {
            switch sniffDropBytes(bytes, contentType: nil) {
            case .submitImage:
                if await submitDroppedImage(bytes) {
                    batch.finish(images: 1, openedPDF: false, failTypes: [])
                    return
                }
            case .openPDF:
                if persistPDF(bytes) {
                    batch.finish(images: 0, openedPDF: true, failTypes: [])
                    return
                }
            default:
                break
            }
        }
        if let pdf, persistPDF(pdf) {
            batch.finish(images: 0, openedPDF: true, failTypes: [])
            return
        }
        if let file {
            if await importDroppedFile(file, batch: batch, types: types) {
                return
            }
        }
        if let web {
            await downloadAndRoute(web, batch: batch, types: types)
            return
        }
        batch.finish(images: 0, openedPDF: false, failTypes: types)
    }

    private func importDroppedFile(_ url: URL, batch: DropBatch, types: [String]) async -> Bool {
        var isDir: ObjCBool = false
        if FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir), isDir.boolValue {
            let before = jobs.count
            await importURLs([url])
            let added = max(jobs.count - before, 0)
            if added > 0 {
                batch.finish(images: added, openedPDF: false, failTypes: [])
                return true
            }
            return false
        }
        if routeImportedURL(url) == .openPDF {
            openPDF(url)
            batch.finish(images: 0, openedPDF: true, failTypes: [])
            return true
        }
        guard let data = try? Data(contentsOf: url), !data.isEmpty else {
            return false
        }
        switch sniffDropBytes(data, contentType: nil) {
        case .submitImage:
            if await submitDroppedImage(data) {
                batch.finish(images: 1, openedPDF: false, failTypes: [])
                return true
            }
        case .openPDF:
            if persistPDF(data) {
                batch.finish(images: 0, openedPDF: true, failTypes: [])
                return true
            }
        default:
            break
        }
        return false
    }

    private func downloadAndRoute(_ url: URL, batch: DropBatch, types: [String]) async {
        do {
            let (data, response) = try await session.data(from: url)
            let contentType = (response as? HTTPURLResponse)?.value(forHTTPHeaderField: "Content-Type")
            switch sniffDropBytes(data, contentType: contentType) {
            case .submitImage:
                if await submitDroppedImage(data) {
                    batch.finish(images: 1, openedPDF: false, failTypes: [])
                    return
                }
            case .openPDF:
                if persistPDF(data) {
                    batch.finish(images: 0, openedPDF: true, failTypes: [])
                    return
                }
            default:
                break
            }
            batch.finish(images: 0, openedPDF: false, failTypes: types)
        } catch {
            batch.finish(images: 0, openedPDF: false, failTypes: types)
        }
    }

    @discardableResult
    private func persistPDF(_ data: Data) -> Bool {
        let dest = FileManager.default.temporaryDirectory
            .appendingPathComponent("drop-\(UUID().uuidString).pdf")
        do {
            try data.write(to: dest, options: .atomic)
            openPDF(dest)
            return true
        } catch {
            banner = "无法保存拖入的 PDF"
            return false
        }
    }

    @discardableResult
    private func submitDroppedImage(_ data: Data) async -> Bool {
        var label = imageUploadLabel(for: data)
        var payload = data
        if label.mime == "application/octet-stream",
           let image = UIImage(data: data),
           let jpeg = image.jpegData(compressionQuality: 0.92)
        {
            payload = jpeg
            label = ImageUploadLabel(filename: "photo.jpg", mime: "image/jpeg")
        }
        return await submitImage(payload, filename: label.filename, mime: label.mime)
    }

    private func makeJobThumbnail(_ data: Data, pointSize: CGFloat = 88) -> Data? {
        guard let image = UIImage(data: data) else { return nil }
        let maxPx = pointSize * 2
        let size = image.size
        let longest = max(size.width, size.height)
        guard longest > 0 else { return nil }
        let ratio = min(1, maxPx / longest)
        let target = CGSize(width: max(size.width * ratio, 1), height: max(size.height * ratio, 1))
        let format = UIGraphicsImageRendererFormat.default()
        format.scale = 1
        let renderer = UIGraphicsImageRenderer(size: target, format: format)
        let thumb = renderer.image { _ in
            image.draw(in: CGRect(origin: .zero, size: target))
        }
        return thumb.jpegData(compressionQuality: 0.72)
    }

    private func mimeFor(_ url: URL) -> String {
        UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? "image/jpeg"
    }

    private func imageFiles(in root: URL, keys: [URLResourceKey]) -> [URL] {
        guard let enumerator = FileManager.default.enumerator(
            at: root,
            includingPropertiesForKeys: keys,
            options: [.skipsHiddenFiles]
        ) else { return [] }
        var files: [URL] = []
        for case let file as URL in enumerator {
            let values = try? file.resourceValues(forKeys: [.isRegularFileKey, .contentTypeKey])
            guard values?.isRegularFile == true else { continue }
            let type = values?.contentType
            let ext = file.pathExtension.lowercased()
            let isImage = type?.conforms(to: .image) == true
                || ["jpg", "jpeg", "png", "webp", "gif", "heic", "tif", "tiff"].contains(ext)
            guard isImage else { continue }
            files.append(file)
        }
        return files
    }
}

@MainActor
final class DropBatch {
    private weak var session: AppSession?
    private var remaining: Int
    private var images = 0
    private var openedPDF = false
    private var failTypes: [String] = []

    init(session: AppSession, remaining: Int) {
        self.session = session
        self.remaining = remaining
    }

    func finish(images: Int, openedPDF: Bool, failTypes: [String]) {
        self.images += images
        if openedPDF { self.openedPDF = true }
        if images == 0 && !openedPDF {
            self.failTypes.append(contentsOf: failTypes)
        }
        remaining -= 1
        guard remaining == 0, let session else { return }
        if self.images > 0 || self.openedPDF {
            session.banner = dropResultBanner(
                images: self.images,
                openedPDF: self.openedPDF,
                typeIdentifiers: []
            )
        } else if session.banner == nil {
            let types = Array(Set(self.failTypes)).sorted()
            session.banner = dropResultBanner(
                images: 0,
                openedPDF: false,
                typeIdentifiers: types.isEmpty ? ["unknown"] : types
            )
        }
    }
}

final class ProviderDropBox: @unchecked Sendable {
    private let lock = NSLock()
    private var pending = 0
    private var bytes: Data?
    private var pdf: Data?
    private var file: URL?
    private var web: URL?
    private var finished = false
    private let types: [String]
    private let session: AppSession
    private let batch: DropBatch

    var pendingCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return pending
    }

    init(types: [String], session: AppSession, batch: DropBatch) {
        self.types = types
        self.session = session
        self.batch = batch
    }

    func addPending() {
        lock.lock()
        pending += 1
        lock.unlock()
    }

    func setBytes(_ data: Data) {
        lock.lock()
        if let existing = bytes {
            let existingGood = looksLikeImage(existing) || looksLikePDF(existing)
            let incomingGood = looksLikeImage(data) || looksLikePDF(data)
            if !existingGood && incomingGood {
                bytes = data
            }
        } else {
            bytes = data
        }
        lock.unlock()
    }

    func setPDF(_ data: Data) {
        lock.lock()
        if pdf == nil || (!looksLikePDF(pdf ?? Data()) && looksLikePDF(data)) {
            pdf = data
        }
        lock.unlock()
    }

    func setFile(_ url: URL) {
        lock.lock()
        if file == nil { file = url }
        lock.unlock()
    }

    func setWeb(_ url: URL) {
        lock.lock()
        if web == nil { web = url }
        lock.unlock()
    }

    func done() {
        lock.lock()
        pending -= 1
        let ready = pending == 0 && !finished
        if ready { finished = true }
        let bytes = bytes
        let pdf = pdf
        let file = file
        let web = web
        let types = types
        lock.unlock()
        guard ready else { return }
        Task { @MainActor in
            await session.resolveDrop(bytes: bytes, pdf: pdf, file: file, web: web, types: types, batch: batch)
        }
    }
}

private func dropImageTypeIdentifiers(for provider: NSItemProvider) -> [String] {
    var ids: [String] = []
    for id in provider.registeredTypeIdentifiers {
        let lower = id.lowercased()
        if let type = UTType(id), type.conforms(to: .image) {
            ids.append(id)
        } else if lower.contains("photos") || lower.contains("heic") || lower.contains("live-photo") {
            ids.append(id)
        }
    }
    let extras = [UTType.image, .jpeg, .png, .gif, .tiff, .webP, .heic].map(\.identifier)
    for extra in extras where provider.hasItemConformingToTypeIdentifier(extra) {
        ids.append(extra)
    }
    return Array(Set(ids))
}

private func dropAnyURL(from item: NSSecureCoding?) -> URL? {
    if let url = item as? URL {
        return url
    }
    if let url = item as? NSURL {
        return url as URL
    }
    if let data = item as? Data {
        if let url = URL(dataRepresentation: data, relativeTo: nil) {
            return url
        }
        if let string = String(data: data, encoding: .utf8) {
            return URL(string: string.trimmingCharacters(in: .whitespacesAndNewlines))
        }
    }
    if let string = item as? String {
        return URL(string: string.trimmingCharacters(in: .whitespacesAndNewlines))
    }
    return nil
}

private func isHTTPURL(_ url: URL) -> Bool {
    let scheme = url.scheme?.lowercased()
    return scheme == "http" || scheme == "https"
}

private func copyDroppedFile(_ url: URL) -> URL {
    let dest = FileManager.default.temporaryDirectory
        .appendingPathComponent("drop-\(UUID().uuidString)-\(url.lastPathComponent)")
    do {
        try FileManager.default.copyItem(at: url, to: dest)
        return dest
    } catch {
        return url
    }
}
