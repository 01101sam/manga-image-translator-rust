import Foundation

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
