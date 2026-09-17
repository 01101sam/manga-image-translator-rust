import Foundation

enum DaemonError: Error, Equatable {
    case unauthorized
    case http(status: Int, message: String)
    case decode
    case transport(String)
}

struct DaemonClient: Sendable {
    var session: URLSession
    var endpoint: DaemonEndpoint
    var token: Token?

    func requestPair() async throws {
        let (_, _) = try await send(
            path: "/pair/request",
            method: "POST",
            body: nil,
            contentType: nil,
            authed: false
        )
    }

    func confirm(code: String) async throws -> Token {
        let payload = try JSONSerialization.data(withJSONObject: ["code": code])
        let (data, _) = try await send(
            path: "/pair/confirm",
            method: "POST",
            body: payload,
            contentType: "application/json",
            authed: false
        )
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let raw = obj["token"] as? String,
              !raw.isEmpty
        else {
            throw DaemonError.decode
        }
        return Token(raw: raw)
    }

    func submitJob(image: Data, filename: String, mime: String, overrides: JobOverrides?) async throws -> String {
        let boundary = "Boundary-\(UUID().uuidString)"
        var body = Data()
        appendPart(to: &body, boundary: boundary, name: "image", filename: filename, mime: mime, data: image)
        if let overrides {
            let obj = overrides.jsonObject
            if !obj.isEmpty, let json = try? JSONSerialization.data(withJSONObject: obj),
               let text = String(data: json, encoding: .utf8)
            {
                appendField(to: &body, boundary: boundary, name: "overrides", value: text)
            }
        }
        body.append("--\(boundary)--\r\n".data(using: .utf8)!)
        let (data, _) = try await send(
            path: "/jobs",
            method: "POST",
            body: body,
            contentType: "multipart/form-data; boundary=\(boundary)",
            authed: true
        )
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let jobId = obj["job_id"] as? String
        else {
            throw DaemonError.decode
        }
        return jobId
    }

    func listJobs() async throws -> [(jobId: String, state: JobLifecycle)] {
        let (data, _) = try await send(path: "/jobs", method: "GET", body: nil, contentType: nil, authed: true)
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let items = obj["jobs"] as? [[String: Any]]
        else {
            throw DaemonError.decode
        }
        return try items.map { item in
            guard let jobId = item["job_id"] as? String,
                  let raw = item["state"] as? String,
                  let state = JobLifecycle(rawValue: raw)
            else {
                throw DaemonError.decode
            }
            return (jobId, state)
        }
    }

    func job(id: String) async throws -> JobSnapshot {
        let (data, _) = try await send(path: "/jobs/\(id)", method: "GET", body: nil, contentType: nil, authed: true)
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let raw = obj["state"] as? String,
              let state = JobLifecycle(rawValue: raw)
        else {
            throw DaemonError.decode
        }
        let position = obj["position_in_queue"] as? Int
        let error = obj["error"] as? String
        return JobSnapshot(jobId: id, state: state, positionInQueue: position, error: error)
    }

    func cancel(id: String) async throws -> JobLifecycle {
        let (data, _) = try await send(
            path: "/jobs/\(id)/cancel",
            method: "POST",
            body: nil,
            contentType: nil,
            authed: true
        )
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let raw = obj["state"] as? String,
              let state = JobLifecycle(rawValue: raw)
        else {
            throw DaemonError.decode
        }
        return state
    }

    func artifact(id: String) async throws -> (Data, String) {
        let (data, mime) = try await send(
            path: "/jobs/\(id)/artifact",
            method: "GET",
            body: nil,
            contentType: nil,
            authed: true
        )
        return (data, mime)
    }

    func engineStatus() async throws -> EngineState {
        let (data, _) = try await send(
            path: "/engine/status",
            method: "GET",
            body: nil,
            contentType: nil,
            authed: true
        )
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let raw = obj["state"] as? String,
              let state = EngineState(rawValue: raw)
        else {
            throw DaemonError.decode
        }
        return state
    }

    private func send(
        path: String,
        method: String,
        body: Data?,
        contentType: String?,
        authed: Bool
    ) async throws -> (Data, String) {
        guard let url = URL(string: path, relativeTo: endpoint.baseURL)?.absoluteURL else {
            throw DaemonError.transport("bad url")
        }
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.httpBody = body
        if let contentType {
            request.setValue(contentType, forHTTPHeaderField: "Content-Type")
        }
        if authed, let token {
            request.setValue("Bearer \(token.raw)", forHTTPHeaderField: "Authorization")
        }
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await session.data(for: request)
        } catch {
            throw DaemonError.transport(error.localizedDescription)
        }
        guard let http = response as? HTTPURLResponse else {
            throw DaemonError.transport("not http")
        }
        if http.statusCode == 401 {
            throw DaemonError.unauthorized
        }
        if !(200...299).contains(http.statusCode) {
            let message = ((try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["error"] as? String)
                ?? HTTPURLResponse.localizedString(forStatusCode: http.statusCode)
            throw DaemonError.http(status: http.statusCode, message: message)
        }
        let mime = http.value(forHTTPHeaderField: "Content-Type") ?? "application/octet-stream"
        return (data, mime)
    }
}

private func appendPart(to body: inout Data, boundary: String, name: String, filename: String, mime: String, data: Data) {
    body.append("--\(boundary)\r\n".data(using: .utf8)!)
    body.append(
        "Content-Disposition: form-data; name=\"\(name)\"; filename=\"\(filename)\"\r\n".data(using: .utf8)!
    )
    body.append("Content-Type: \(mime)\r\n\r\n".data(using: .utf8)!)
    body.append(data)
    body.append("\r\n".data(using: .utf8)!)
}

private func appendField(to body: inout Data, boundary: String, name: String, value: String) {
    body.append("--\(boundary)\r\n".data(using: .utf8)!)
    body.append("Content-Disposition: form-data; name=\"\(name)\"\r\n\r\n".data(using: .utf8)!)
    body.append(value.data(using: .utf8)!)
    body.append("\r\n".data(using: .utf8)!)
}
