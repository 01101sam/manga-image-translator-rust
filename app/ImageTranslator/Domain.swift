import Foundation
import UniformTypeIdentifiers

struct DaemonEndpoint: Equatable, Hashable, Sendable {
    var name: String
    var host: String
    var port: UInt16

    var baseURL: URL {
        let host = stripInterfaceZone(self.host)
        let hostPart = host.contains(":") && !host.contains(".") ? "[\(host)]" : host
        if let url = URL(string: "http://\(hostPart):\(port)") {
            return url
        }
        return URL(fileURLWithPath: "/")
    }
}

func stripInterfaceZone(_ host: String) -> String {
    guard let idx = host.firstIndex(of: "%") else { return host }
    return String(host[..<idx])
}

struct Token: Equatable, Hashable, Sendable {
    var raw: String
}

struct PairedDaemon: Equatable, Sendable {
    var endpoint: DaemonEndpoint
    var token: Token
}

enum PairingPhase: Equatable, Sendable {
    case browsing
    case requesting(DaemonEndpoint)
    case awaitingCode(DaemonEndpoint)
    case confirming(DaemonEndpoint, code: String)
    case paired(DaemonEndpoint, Token)
    case failed(DaemonEndpoint, message: String)
    case unauthorized(DaemonEndpoint)

    var endpoint: DaemonEndpoint? {
        switch self {
        case .browsing:
            return nil
        case .requesting(let endpoint),
             .awaitingCode(let endpoint),
             .confirming(let endpoint, _),
             .paired(let endpoint, _),
             .failed(let endpoint, _),
             .unauthorized(let endpoint):
            return endpoint
        }
    }

    var token: Token? {
        if case .paired(_, let token) = self {
            return token
        }
        return nil
    }
}

enum PairingEvent: Equatable, Sendable {
    case select(DaemonEndpoint)
    case requestSucceeded
    case requestFailed(String)
    case submitCode(String)
    case confirmSucceeded(Token)
    case confirmFailed(String)
    case unauthorized
    case retry
    case forget
}

enum PairingCommand: Equatable, Sendable {
    case none
    case requestPair(DaemonEndpoint)
    case confirm(DaemonEndpoint, code: String)
    case persist(PairedDaemon)
    case clearToken
}

func reducePairing(_ phase: PairingPhase, _ event: PairingEvent) -> (PairingPhase, PairingCommand) {
    switch (phase, event) {
    case (_, .select(let endpoint)):
        return (.requesting(endpoint), .requestPair(endpoint))
    case (.requesting(let endpoint), .requestSucceeded):
        return (.awaitingCode(endpoint), .none)
    case (.requesting(let endpoint), .requestFailed(let message)):
        return (.failed(endpoint, message: message), .none)
    case (.awaitingCode(let endpoint), .submitCode(let code)):
        if code.count == 6, code.allSatisfy(\.isNumber) {
            return (.confirming(endpoint, code: code), .confirm(endpoint, code: code))
        }
        return (.failed(endpoint, message: "请输入6位数字"), .none)
    case (.confirming(let endpoint, _), .confirmSucceeded(let token)):
        return (.paired(endpoint, token), .persist(PairedDaemon(endpoint: endpoint, token: token)))
    case (.confirming(let endpoint, _), .confirmFailed(let message)):
        return (.failed(endpoint, message: message), .none)
    case (_, .unauthorized):
        if let endpoint = phase.endpoint {
            return (.unauthorized(endpoint), .clearToken)
        }
        return (phase, .clearToken)
    case (.failed(let endpoint, _), .retry), (.unauthorized(let endpoint), .retry):
        return (.requesting(endpoint), .requestPair(endpoint))
    case (_, .forget):
        return (.browsing, .clearToken)
    default:
        return (phase, .none)
    }
}

enum JobLifecycle: String, Equatable, Sendable {
    case queued
    case running
    case done
    case failed
    case cancelled

    var isActive: Bool {
        self == .queued || self == .running
    }
}

struct JobSnapshot: Equatable, Sendable {
    var jobId: String
    var state: JobLifecycle
    var positionInQueue: Int?
    var error: String?
}

struct JobRow: Equatable, Sendable {
    var snapshot: JobSnapshot
    var cancelPending: Bool
    var thumbnail: Data? = nil
}

enum JobEvent: Equatable, Sendable {
    case snapshot(JobSnapshot)
    case requestCancel
    case cancelAccepted
    case cancelRejected
}

enum JobCommand: Equatable, Sendable {
    case none
    case cancel(jobId: String)
}

func canCancelJob(_ state: JobLifecycle) -> Bool {
    state == .queued
}

func reduceJob(_ row: JobRow, _ event: JobEvent) -> (JobRow, JobCommand) {
    switch event {
    case .snapshot(let snap):
        let pending = row.cancelPending && snap.state == .queued
        return (JobRow(snapshot: snap, cancelPending: pending, thumbnail: row.thumbnail), .none)
    case .requestCancel:
        guard canCancelJob(row.snapshot.state), !row.cancelPending else {
            return (row, .none)
        }
        return (JobRow(snapshot: row.snapshot, cancelPending: true, thumbnail: row.thumbnail), .cancel(jobId: row.snapshot.jobId))
    case .cancelAccepted:
        var snap = row.snapshot
        snap.state = .cancelled
        snap.positionInQueue = nil
        return (JobRow(snapshot: snap, cancelPending: false, thumbnail: row.thumbnail), .none)
    case .cancelRejected:
        return (JobRow(snapshot: row.snapshot, cancelPending: false, thumbnail: row.thumbnail), .none)
    }
}

enum PageTranslation: Equatable, Sendable {
    case untranslated
    case queued
    case running
    case done
}

enum PageViewing: Equatable, Sendable {
    case original
    case translated
}

struct ReaderPage: Equatable, Sendable {
    var index: Int
    var translation: PageTranslation
    var viewing: PageViewing
    var jobId: String?
}

enum PageEvent: Equatable, Sendable {
    case space
    case submitted(jobId: String)
    case jobState(JobLifecycle)
}

enum PageCommand: Equatable, Sendable {
    case none
    case enqueue
    case toggleView
}

func reducePage(_ page: ReaderPage, _ event: PageEvent) -> (ReaderPage, PageCommand) {
    switch event {
    case .space:
        switch page.translation {
        case .untranslated:
            return (page, .enqueue)
        case .done:
            var next = page
            next.viewing = page.viewing == .original ? .translated : .original
            return (next, .toggleView)
        case .queued, .running:
            return (page, .none)
        }
    case .submitted(let jobId):
        var next = page
        next.translation = .queued
        next.jobId = jobId
        return (next, .none)
    case .jobState(let life):
        var next = page
        switch life {
        case .queued:
            next.translation = .queued
        case .running:
            next.translation = .running
        case .done:
            next.translation = .done
        case .failed, .cancelled:
            next.translation = .untranslated
            next.jobId = nil
            next.viewing = .original
        }
        return (next, .none)
    }
}

func untranslatedIndexes(_ pages: [ReaderPage]) -> [Int] {
    pages.filter { $0.translation == .untranslated }.map(\.index)
}

enum EngineState: String, Equatable, Sendable {
    case stopped
    case starting
    case running
    case draining
}

struct ImageUploadLabel: Equatable, Sendable {
    var filename: String
    var mime: String
}

func imageUploadLabel(for data: Data) -> ImageUploadLabel {
    let png: [UInt8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
    if data.count >= png.count, data.prefix(png.count).elementsEqual(png) {
        return ImageUploadLabel(filename: "photo.png", mime: "image/png")
    }
    if data.count >= 3, data[data.startIndex] == 0xFF, data[data.startIndex + 1] == 0xD8, data[data.startIndex + 2] == 0xFF {
        return ImageUploadLabel(filename: "photo.jpg", mime: "image/jpeg")
    }
    return ImageUploadLabel(filename: "photo.bin", mime: "application/octet-stream")
}

struct JobOverrides: Equatable, Sendable {
    var targetLang: String?
    var detector: String?
    var translator: String?

    var jsonObject: [String: String] {
        var obj: [String: String] = [:]
        if let targetLang { obj["target_lang"] = targetLang }
        if let detector { obj["detector"] = detector }
        if let translator { obj["translator"] = translator }
        return obj
    }
}

enum DropAction: Equatable, Sendable {
    case submitImage
    case openPDF
    case importURL
    case downloadWeb
    case ignore
    case ignoreWithMessage(String)
}

func routeDrop(type: UTType, hasData: Bool) -> DropAction {
    if type.conforms(to: .fileURL) {
        return .importURL
    }
    if type.conforms(to: .url) {
        return .downloadWeb
    }
    if type.conforms(to: .pdf) {
        return hasData ? .openPDF : .ignore
    }
    if type.conforms(to: .image) {
        return hasData ? .submitImage : .ignore
    }
    if isPhotoAssetType(type) {
        return hasData ? .submitImage : .ignore
    }
    return .ignoreWithMessage(unrecognizedDropMessage(typeIdentifiers: [type.identifier]))
}

func routeImportedURL(_ url: URL) -> DropAction {
    url.pathExtension.lowercased() == "pdf" ? .openPDF : .importURL
}

func isPhotoAssetType(_ type: UTType) -> Bool {
    let id = type.identifier.lowercased()
    return id.contains("photos") || id.contains("live-photo")
}

func unrecognizedDropMessage(typeIdentifiers: [String]) -> String {
    "无法识别拖入内容: [\(typeIdentifiers.joined(separator: ", "))]"
}

func dropResultBanner(images: Int, openedPDF: Bool, typeIdentifiers: [String]) -> String {
    if images > 0 && openedPDF {
        return "已导入 \(images) 张图片 · 已打开 PDF"
    }
    if images > 0 {
        return "已导入 \(images) 张图片"
    }
    if openedPDF {
        return "已打开 PDF"
    }
    return unrecognizedDropMessage(typeIdentifiers: typeIdentifiers)
}

func sniffDropBytes(_ data: Data, contentType: String?) -> DropAction {
    let mime = contentType?
        .split(separator: ";")
        .first?
        .trimmingCharacters(in: .whitespaces)
        .lowercased()
    if looksLikePDF(data) || mime == "application/pdf" {
        return .openPDF
    }
    if looksLikeImage(data) || (mime?.hasPrefix("image/") ?? false) {
        return .submitImage
    }
    return .ignoreWithMessage(unrecognizedDropMessage(typeIdentifiers: [mime ?? "unknown"]))
}

func looksLikePDF(_ data: Data) -> Bool {
    data.starts(with: Data("%PDF".utf8))
}

func looksLikeImage(_ data: Data) -> Bool {
    if data.count >= 3,
       data[data.startIndex] == 0xFF,
       data[data.startIndex + 1] == 0xD8,
       data[data.startIndex + 2] == 0xFF
    {
        return true
    }
    let png: [UInt8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
    if data.count >= png.count, data.prefix(png.count).elementsEqual(png) {
        return true
    }
    if data.starts(with: Data("GIF87a".utf8)) || data.starts(with: Data("GIF89a".utf8)) {
        return true
    }
    if data.count >= 12,
       data.starts(with: Data("RIFF".utf8)),
       data[8..<12].elementsEqual(Data("WEBP".utf8))
    {
        return true
    }
    if data.count >= 12, data[4..<8].elementsEqual(Data("ftyp".utf8)) {
        return true
    }
    return false
}

func shortJobId(_ jobId: String) -> String {
    String(jobId.split(separator: "-").first ?? Substring(jobId))
}

func moveJobHighlight(count: Int, current: Int?, delta: Int) -> Int? {
    guard count > 0 else { return nil }
    let base = current ?? (delta > 0 ? -1 : count)
    return min(max(base + delta, 0), count - 1)
}

func adjacentDoneJobId(jobs: [JobRow], currentId: String?, delta: Int) -> String? {
    let done = jobs.filter { $0.snapshot.state == .done }
    guard !done.isEmpty else { return nil }
    if let currentId, let idx = done.firstIndex(where: { $0.snapshot.jobId == currentId }) {
        let next = idx + delta
        if done.indices.contains(next) {
            return done[next].snapshot.jobId
        }
        return currentId
    }
    return delta >= 0 ? done.first?.snapshot.jobId : done.last?.snapshot.jobId
}

func canOpenJobPreview(_ state: JobLifecycle) -> Bool {
    state == .done
}

enum JobListArrow: Equatable, Sendable {
    case up
    case down
    case left
    case right
}

func jobListArrowDelta(_ arrow: JobListArrow, previewOpen: Bool) -> Int? {
    switch arrow {
    case .up:
        return -1
    case .down:
        return 1
    case .left:
        return previewOpen ? -1 : nil
    case .right:
        return previewOpen ? 1 : nil
    }
}

func removeLocalJobs(jobs: inout [JobRow], artifacts: inout [String: Data], at offsets: IndexSet) {
    let ids = offsets.compactMap { jobs.indices.contains($0) ? jobs[$0].snapshot.jobId : nil }
    jobs.remove(atOffsets: offsets)
    for id in ids {
        artifacts.removeValue(forKey: id)
    }
}

func moveLocalJobs(jobs: inout [JobRow], from source: IndexSet, to destination: Int) {
    jobs.move(fromOffsets: source, toOffset: destination)
}
