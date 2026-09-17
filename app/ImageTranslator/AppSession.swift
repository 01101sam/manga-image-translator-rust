import Combine
import Foundation
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

    let tokens: TokenStoring
    let browser: BonjourBrowser
    private let session: URLSession
    private var pollTask: Task<Void, Never>?
    private var browserBag = Set<AnyCancellable>()
    private var pendingPDF: URL?
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

    func submitImage(_ data: Data, filename: String, mime: String = "image/jpeg") async {
        banner = nil
        guard var api = client, api.token != nil else { return }
        do {
            let jobId = try await api.submitJob(image: data, filename: filename, mime: mime, overrides: nil)
            let row = JobRow(
                snapshot: JobSnapshot(jobId: jobId, state: .queued, positionInQueue: nil, error: nil),
                cancelPending: false
            )
            jobs.insert(row, at: 0)
        } catch DaemonError.unauthorized {
            applyPairing(.unauthorized)
        } catch {
            await refreshEngineHint(fallback: error.localizedDescription)
        }
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
            if url.pathExtension.lowercased() == "pdf" {
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
