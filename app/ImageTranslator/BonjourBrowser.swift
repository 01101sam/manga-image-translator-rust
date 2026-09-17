import Foundation
import Network

@MainActor
final class BonjourBrowser: ObservableObject {
    @Published private(set) var daemons: [DaemonEndpoint] = []

    private var browser: NWBrowser?
    private var resolvers: [ObjectIdentifier: NWConnection] = [:]

    func start() {
        stop()
        let params = NWParameters()
        params.includePeerToPeer = true
        let browser = NWBrowser(for: .bonjour(type: "_imagetranslator._tcp", domain: nil), using: params)
        browser.browseResultsChangedHandler = { [weak self] results, _ in
            Task { @MainActor in
                self?.ingest(results)
            }
        }
        browser.start(queue: .main)
        self.browser = browser
    }

    func stop() {
        browser?.cancel()
        browser = nil
        resolvers.values.forEach { $0.cancel() }
        resolvers.removeAll()
        daemons = []
    }

    private func ingest(_ results: Set<NWBrowser.Result>) {
        for result in results {
            resolve(result)
        }
        let names = Set(results.compactMap { name(of: $0.endpoint) })
        daemons.removeAll { !names.contains($0.name) }
    }

    private func name(of endpoint: NWEndpoint) -> String? {
        if case .service(let name, _, _, _) = endpoint {
            return name
        }
        return nil
    }

    private func resolve(_ result: NWBrowser.Result) {
        if case .hostPort(let host, let port) = result.endpoint {
            upsert(DaemonEndpoint(name: hostLabel(host), host: hostLabel(host), port: port.rawValue))
            return
        }
        let connection = NWConnection(to: result.endpoint, using: .tcp)
        let id = ObjectIdentifier(connection)
        resolvers[id] = connection
        connection.stateUpdateHandler = { [weak self] state in
            Task { @MainActor in
                guard let self else { return }
                if case .ready = state, let remote = connection.currentPath?.remoteEndpoint,
                   case .hostPort(let host, let port) = remote
                {
                    let serviceName = self.name(of: result.endpoint) ?? self.hostLabel(host)
                    self.upsert(DaemonEndpoint(name: serviceName, host: self.hostLabel(host), port: port.rawValue))
                }
                if case .ready = state {
                    connection.cancel()
                    self.resolvers[id] = nil
                }
                if case .failed = state {
                    connection.cancel()
                    self.resolvers[id] = nil
                }
            }
        }
        connection.start(queue: .main)
    }

    private func upsert(_ endpoint: DaemonEndpoint) {
        if let idx = daemons.firstIndex(where: { $0.name == endpoint.name }) {
            daemons[idx] = endpoint
        } else {
            daemons.append(endpoint)
            daemons.sort { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        }
    }

    private func hostLabel(_ host: NWEndpoint.Host) -> String {
        switch host {
        case .name(let name, _):
            return name
        case .ipv4(let addr):
            return "\(addr)"
        case .ipv6(let addr):
            return "\(addr)"
        @unknown default:
            return "\(host)"
        }
    }
}
