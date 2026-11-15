import Combine
import Foundation

/// 配对状态: Bonjour 发现的 Daemon 加上 Keychain 里永久保存的 Token。
@MainActor
final class HostSession {
    private let tokens: TokenStoring
    private let browser: BonjourBrowser
    private var subscription: AnyCancellable?
    private var lastSent: [DaemonEndpoint] = []

    /// 发现的 Daemon 列表变了 (全量)。
    var onDaemonsChanged: (([[String: Any]]) -> Void)?

    init(tokens: TokenStoring, browser: BonjourBrowser? = nil) {
        self.tokens = tokens
        self.browser = browser ?? BonjourBrowser()
    }

    /// 测试钩子给了 Daemon 地址时只播种, 不启动 Bonjour: 启动后 BonjourBrowser 会把不在浏览结果里的条目清掉。
    func start(seed: DaemonEndpoint?) {
        subscription = browser.$daemons.sink { [weak self] daemons in
            guard let self, daemons != lastSent else { return }
            lastSent = daemons
            onDaemonsChanged?(daemons.map(Self.json))
        }
        if let seed {
            browser.seed(seed)
        } else {
            browser.start()
        }
    }

    func snapshot() -> [String: Any] {
        let paired: Any
        if let pairedDaemon = tokens.load() {
            paired = ["daemon": Self.json(pairedDaemon.endpoint), "token": pairedDaemon.token.raw]
        } else {
            paired = NSNull()
        }
        return ["daemons": browser.daemons.map(Self.json), "paired": paired]
    }

    func saveToken(_ data: Any?) throws {
        guard let args = data as? [String: Any], let daemon = args["daemon"] as? [String: Any],
              let name = daemon["name"] as? String, let url = daemon["url"] as? String,
              let token = args["token"] as? String, !token.isEmpty,
              let endpoint = DaemonEndpoint(name: name, url: url)
        else { throw HostCallError("配对信息不完整, 没有保存") }
        tokens.save(PairedDaemon(endpoint: endpoint, token: Token(raw: token)))
    }

    func clearToken() {
        tokens.clear()
    }

    static func json(_ endpoint: DaemonEndpoint) -> [String: Any] {
        ["name": endpoint.name, "url": endpoint.baseURL.absoluteString]
    }
}

extension DaemonEndpoint {
    /// 从 TSX 给的 { name, url } 还原; Daemon 只说 http, url 只认 http://host[:port]。
    init?(name: String, url: String) {
        guard let parsed = URL(string: url), parsed.scheme == "http",
              let host = parsed.host, !host.isEmpty
        else { return nil }
        let port = parsed.port ?? 80
        guard (1...65535).contains(port) else { return nil }
        self.init(name: name, host: host, port: UInt16(port))
    }
}
