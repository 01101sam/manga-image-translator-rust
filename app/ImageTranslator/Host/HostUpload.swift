import Foundation

/// 与 HOST_PROTOCOL.md 的 upload.start 失败 kind 一一对应。
enum UploadFailure: String, Sendable {
    case queueFull = "queue-full"
    case engineStopped = "engine-stopped"
    case unauthorized
    case rejected
    case network
}

enum UploadOutcome: Equatable, Sendable {
    case accepted(jobID: String)
    case failed(UploadFailure, message: String)

    var json: [String: Any] {
        switch self {
        case .accepted(let jobID): return ["ok": true, "jobId": jobID]
        case .failed(let kind, let message): return ["ok": false, "kind": kind.rawValue, "message": message]
        }
    }

    /// Daemon 的应答映射成 kind。表之外的 4xx 当作拒绝: 自动重试不会让它变好。
    static func from(status: Int, body: Data) -> UploadOutcome {
        let json = try? JSONSerialization.jsonObject(with: body) as? [String: Any]
        if status == 200 {
            if let jobID = json?["job_id"] as? String, !jobID.isEmpty { return .accepted(jobID: jobID) }
            return .failed(.network, message: "Daemon 应答里没有 job_id")
        }
        let message = (json?["error"] as? String) ?? "HTTP \(status)"
        switch status {
        case 401: return .failed(.unauthorized, message: message)
        case 429: return .failed(.queueFull, message: message)
        case 503: return .failed(.engineStopped, message: message)
        case 400, 413, 415, 422: return .failed(.rejected, message: message)
        case 500...599: return .failed(.network, message: message)
        default: return .failed(.rejected, message: message)
        }
    }
}

enum Multipart {
    static let chunkSize = 1 << 20

    /// 先写头, 再分块拷贝文件, 最后写尾。每块放在自己的 autoreleasepool 里, 内存里同时只有一块, 不会因为文件大而涨。
    static func writeBody(to body: URL, boundary: String, file: URL, filename: String, mime: String, overrides: String) throws {
        FileManager.default.createFile(atPath: body.path, contents: nil)
        let output = try FileHandle(forWritingTo: body)
        defer { try? output.close() }
        let head = "--\(boundary)\r\nContent-Disposition: form-data; name=\"image\"; filename=\"\(quoted(filename))\"\r\nContent-Type: \(mime)\r\n\r\n"
        try output.write(contentsOf: Data(head.utf8))
        let input = try FileHandle(forReadingFrom: file)
        defer { try? input.close() }
        while true {
            let finished: Bool = try autoreleasepool {
                guard let chunk = try input.read(upToCount: chunkSize), !chunk.isEmpty else { return true }
                try output.write(contentsOf: chunk)
                return false
            }
            if finished { break }
        }
        let tail = "\r\n--\(boundary)\r\nContent-Disposition: form-data; name=\"overrides\"\r\n\r\n\(overrides)\r\n--\(boundary)--\r\n"
        try output.write(contentsOf: Data(tail.utf8))
    }

    /// 头里的引号和换行会破坏 multipart 的结构。
    static func quoted(_ name: String) -> String {
        name.replacingOccurrences(of: "\"", with: "%22")
            .replacingOccurrences(of: "\r", with: "_")
            .replacingOccurrences(of: "\n", with: "_")
    }
}

struct UploadRequest: Sendable {
    var file: URL
    var filename: String
    var mime: String
    var endpoint: URL
    var token: String
    var overrides: String
}

/// upload.start: multipart 体先落到临时文件, 再 uploadTask(fromFile:) 流式上传, 字节不进 JS 也不在内存里拼 Data。
final class UploadClient: @unchecked Sendable {
    private let session: URLSession
    private let files: ManagedFiles

    init(files: ManagedFiles) {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.waitsForConnectivity = false
        configuration.timeoutIntervalForRequest = 120
        configuration.timeoutIntervalForResource = 1800
        configuration.httpMaximumConnectionsPerHost = 4
        self.session = URLSession(configuration: configuration)
        self.files = files
    }

    func upload(_ request: UploadRequest) async -> UploadOutcome {
        guard request.endpoint.scheme == "http" || request.endpoint.scheme == "https" else {
            return .failed(.rejected, message: "上传地址不是 http 地址")
        }
        let boundary = "it-\(UUID().uuidString)"
        let body = files.scratchFile(named: "upload.body")
        defer { try? FileManager.default.removeItem(at: body) }
        do {
            try Multipart.writeBody(
                to: body, boundary: boundary, file: request.file, filename: request.filename,
                mime: request.mime, overrides: request.overrides
            )
            var urlRequest = URLRequest(url: request.endpoint)
            urlRequest.httpMethod = "POST"
            urlRequest.setValue("multipart/form-data; boundary=\(boundary)", forHTTPHeaderField: "Content-Type")
            urlRequest.setValue("Bearer \(request.token)", forHTTPHeaderField: "Authorization")
            let (data, response) = try await session.upload(for: urlRequest, fromFile: body)
            return UploadOutcome.from(status: (response as? HTTPURLResponse)?.statusCode ?? 0, body: data)
        } catch is CancellationError {
            return .failed(.network, message: "上传已取消")
        } catch let error as URLError {
            return .failed(.network, message: error.localizedDescription)
        } catch {
            return .failed(.network, message: "读写临时文件失败: \(error.localizedDescription)")
        }
    }
}
