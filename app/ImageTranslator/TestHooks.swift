import Foundation

struct TestHooks: Equatable, Sendable {
    var paired: PairedDaemon?
    var openPDF: String?
    var autoSubmitImage: String?
    var autoImportFolder: String?
    var daemonHost: String?
    var daemonPort: UInt16?

    var autoSubmitPaths: [String] {
        (autoSubmitImage ?? "")
            .split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }

    var browseDaemon: DaemonEndpoint? {
        guard let daemonPort else { return nil }
        return DaemonEndpoint(name: "Daemon", host: daemonHost ?? "127.0.0.1", port: daemonPort)
    }

    var isolatesSession: Bool {
        paired != nil || browseDaemon != nil || openPDF != nil || autoSubmitImage != nil || autoImportFolder != nil
    }

    static func fromProcessInfo() -> TestHooks {
        parse(ProcessInfo.processInfo.arguments)
    }

    static func parse(_ args: [String]) -> TestHooks {
        var hooks = TestHooks()
        var i = 0
        while i < args.count {
            let flag = args[i]
            if flag == "-paired-test", i + 3 < args.count, let port = UInt16(args[i + 2]) {
                hooks.paired = PairedDaemon(
                    endpoint: DaemonEndpoint(name: "Daemon", host: args[i + 1], port: port),
                    token: Token(raw: args[i + 3])
                )
                i += 4
                continue
            }
            if flag == "-open-test-pdf", i + 1 < args.count {
                hooks.openPDF = args[i + 1]
                i += 2
                continue
            }
            if flag == "-auto-submit-image", i + 1 < args.count {
                hooks.autoSubmitImage = args[i + 1]
                i += 2
                continue
            }
            if flag == "-auto-import-folder", i + 1 < args.count {
                hooks.autoImportFolder = args[i + 1]
                i += 2
                continue
            }
            if flag == "-daemon-host", i + 1 < args.count {
                hooks.daemonHost = args[i + 1]
                i += 2
                continue
            }
            if flag == "-daemon-port", i + 1 < args.count, let port = UInt16(args[i + 1]) {
                hooks.daemonPort = port
                i += 2
                continue
            }
            i += 1
        }
        return hooks
    }

    func resolveFile(_ path: String) -> URL? {
        let url = URL(fileURLWithPath: path)
        let ext = url.pathExtension
        let name = url.deletingPathExtension().lastPathComponent
        if let bundled = Bundle.main.url(forResource: name, withExtension: ext.isEmpty ? nil : ext) {
            return bundled
        }
        if path.hasPrefix("/"), FileManager.default.isReadableFile(atPath: url.path) {
            return url
        }
        return nil
    }

    func resolveFolder(_ path: String) -> URL? {
        let url = URL(fileURLWithPath: path)
        if let bundled = Bundle.main.url(forResource: url.lastPathComponent, withExtension: nil) {
            var isDir: ObjCBool = false
            if FileManager.default.fileExists(atPath: bundled.path, isDirectory: &isDir), isDir.boolValue {
                return bundled
            }
        }
        if let resource = Bundle.main.resourceURL?.appendingPathComponent(url.lastPathComponent) {
            var isDir: ObjCBool = false
            if FileManager.default.fileExists(atPath: resource.path, isDirectory: &isDir), isDir.boolValue {
                return resource
            }
        }
        var isDir: ObjCBool = false
        if path.hasPrefix("/"), FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir), isDir.boolValue {
            return url
        }
        return nil
    }
}
