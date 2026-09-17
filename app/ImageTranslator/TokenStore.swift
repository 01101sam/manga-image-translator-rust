import Foundation
import Security

protocol TokenStoring: AnyObject {
    func load() -> PairedDaemon?
    func save(_ paired: PairedDaemon)
    func clear()
}

final class KeychainTokenStore: TokenStoring {
    private let service = "com.wisdomworldworkshop.imagetranslator"
    private let account = "pairing"

    func load() -> PairedDaemon? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess, let data = item as? Data else {
            return nil
        }
        return decode(data)
    }

    func save(_ paired: PairedDaemon) {
        clear()
        let data = encode(paired)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlock,
        ]
        SecItemAdd(query as CFDictionary, nil)
    }

    func clear() {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(query as CFDictionary)
    }

    private func encode(_ paired: PairedDaemon) -> Data {
        let obj: [String: Any] = [
            "name": paired.endpoint.name,
            "host": paired.endpoint.host,
            "port": Int(paired.endpoint.port),
            "token": paired.token.raw,
        ]
        return (try? JSONSerialization.data(withJSONObject: obj)) ?? Data()
    }

    private func decode(_ data: Data) -> PairedDaemon? {
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let name = obj["name"] as? String,
              let host = obj["host"] as? String,
              let port = obj["port"] as? Int,
              let token = obj["token"] as? String,
              (1...65535).contains(port),
              !token.isEmpty
        else {
            return nil
        }
        return PairedDaemon(
            endpoint: DaemonEndpoint(name: name, host: host, port: UInt16(port)),
            token: Token(raw: token)
        )
    }
}

final class MemoryTokenStore: TokenStoring {
    private var value: PairedDaemon?

    init(value: PairedDaemon? = nil) {
        self.value = value
    }

    func load() -> PairedDaemon? { value }
    func save(_ paired: PairedDaemon) { value = paired }
    func clear() { value = nil }
}
