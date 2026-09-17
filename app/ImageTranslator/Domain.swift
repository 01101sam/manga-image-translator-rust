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
        return (JobRow(snapshot: snap, cancelPending: pending), .none)
    case .requestCancel:
        guard canCancelJob(row.snapshot.state), !row.cancelPending else {
            return (row, .none)
        }
        return (JobRow(snapshot: row.snapshot, cancelPending: true), .cancel(jobId: row.snapshot.jobId))
    case .cancelAccepted:
        var snap = row.snapshot
        snap.state = .cancelled
        snap.positionInQueue = nil
        return (JobRow(snapshot: snap, cancelPending: false), .none)
    case .cancelRejected:
        return (JobRow(snapshot: row.snapshot, cancelPending: false), .none)
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
    case ignore
}

func routeDrop(type: UTType, hasData: Bool) -> DropAction {
    if type.conforms(to: .fileURL) {
        return .importURL
    }
    if type.conforms(to: .pdf) {
        return hasData ? .openPDF : .ignore
    }
    if type.conforms(to: .image) {
        return hasData ? .submitImage : .ignore
    }
    return .ignore
}

func routeImportedURL(_ url: URL) -> DropAction {
    url.pathExtension.lowercased() == "pdf" ? .openPDF : .importURL
}
