import Foundation

struct TestHooks: Equatable, Sendable {
    var paired: PairedDaemon?
    var autoImportFolder: String?
    /// -auto-import-urls: 与 -auto-import-folder 同一个目录, 但走文件选择器的 URL 拷贝路径, 不经过 NSItemProvider。
    var autoImportViaURLs = false
    /// files.pick 不弹选择器, 直接从这个目录取图片 (UI 测试没法驱动系统选择器)。
    var pickFolder: String?
    /// pdf.pick 不弹选择器, 直接用这个 PDF。
    var pickPDF: String?
    /// -inbox-dir: 分享扩展的收件箱换成这个目录, UI 测试不碰 App Group 容器里用户自己的东西。
    var inboxDirectory: String?
    /// -seed-pasteboard: 启动时把这个文件 (或目录里的图片和 PDF) 放进系统剪贴板, UI 测试用来验证粘贴。
    var seedPasteboard: String?
    /// -debug-drag-source: 窗口里放一个可拖的小方块, UI 测试用它发起真正的拖放。
    var debugDragSource: String?
    /// 加载这个 bundle 文件而不是 App 里内置的 (探针页用)。
    var bundlePath: String?
    /// 起始分区, 传给 TSX 的 host.params.section。
    var section: String?
    /// 任务页的排版, 传给 TSX 的 host.params.layout; "list" 在 iPad 上也排成一行一项 (手机的样子)。
    var layout: String?
    /// 启动时先清掉 Keychain 里的配对, 让 UI 测试从未配对开始。
    var forgetPairing = false
    var daemonHost: String?
    var daemonPort: UInt16?

    var browseDaemon: DaemonEndpoint? {
        guard let daemonPort else { return nil }
        return DaemonEndpoint(name: "Daemon", host: daemonHost ?? "127.0.0.1", port: daemonPort)
    }

    /// Only `-paired-test` swaps in MemoryTokenStore. Host/port and file hooks stay on Keychain.
    var isolatesSession: Bool {
        paired != nil
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
            if flag == "-auto-import-folder", i + 1 < args.count {
                hooks.autoImportFolder = args[i + 1]
                i += 2
                continue
            }
            if flag == "-auto-import-urls" {
                hooks.autoImportViaURLs = true
                i += 1
                continue
            }
            if flag == "-pick-folder", i + 1 < args.count {
                hooks.pickFolder = args[i + 1]
                i += 2
                continue
            }
            if flag == "-pick-pdf", i + 1 < args.count {
                hooks.pickPDF = args[i + 1]
                i += 2
                continue
            }
            if flag == "-debug-drag-source", i + 1 < args.count {
                hooks.debugDragSource = args[i + 1]
                i += 2
                continue
            }
            if flag == "-inbox-dir", i + 1 < args.count {
                hooks.inboxDirectory = args[i + 1]
                i += 2
                continue
            }
            if flag == "-seed-pasteboard", i + 1 < args.count {
                hooks.seedPasteboard = args[i + 1]
                i += 2
                continue
            }
            if flag == "-bundle", i + 1 < args.count {
                hooks.bundlePath = args[i + 1]
                i += 2
                continue
            }
            if flag == "-section", i + 1 < args.count {
                hooks.section = args[i + 1]
                i += 2
                continue
            }
            if flag == "-layout", i + 1 < args.count {
                hooks.layout = args[i + 1]
                i += 2
                continue
            }
            if flag == "-forget-pairing" {
                hooks.forgetPairing = true
                i += 1
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
}