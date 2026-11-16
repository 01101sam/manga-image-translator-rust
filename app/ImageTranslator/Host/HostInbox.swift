import Foundation

/// App Group 收件箱 (分享扩展放进去的) 到沙盒。收件箱的布局见 ShareInbox。
///
/// 一个条目只处理一次, 处理完或处理不了, 整个条目目录都删掉。可以在任何时刻崩溃再重来而不重复也不丢:
/// 搬文件是同一个卷上的 rename, 文件要么还在收件箱要么已经在沙盒里, 没有中间状态; 搬完到删目录之间崩溃, 留下的是
/// 空目录, 下次直接删。(rename 失败退回拷贝时, 拷完到删目录之间崩溃会重复一次, 同一个卷上不会走到。)
/// 用 actor 是为了进程里一次只有一个在搬: 多个窗口、前台通知接连触发时, 同一个条目不会被两边同时处理。
actor InboxDrainer {
    /// 测试钩子 -inbox-dir 换掉真正的 App Group 容器, 免得 UI 测试和单测动用户的收件箱。
    static let shared: InboxDrainer = {
        let override = TestHooks.fromProcessInfo().inboxDirectory
        return InboxDrainer(inbox: override.map { URL(fileURLWithPath: $0, isDirectory: true) } ?? ShareInbox.directory())
    }()

    private let inbox: URL?

    /// inbox 为 nil 表示取不到 App Group 容器 (没有授权): 没有可搬的东西。
    init(inbox: URL?) {
        self.inbox = inbox
    }

    func drain(using ingestor: FileIngestor) async -> IngestResult {
        guard let inbox else { return IngestResult() }
        ShareInbox.removeStaleStaging(in: inbox)
        var result = IngestResult()
        for entry in ShareInbox.entries(in: inbox) {
            result.merge(await importEntry(entry, using: ingestor))
        }
        return result
    }

    private func importEntry(_ entry: URL, using ingestor: FileIngestor) async -> IngestResult {
        defer { try? FileManager.default.removeItem(at: entry) }
        guard let payload = ShareInbox.payload(of: entry) else { return IngestResult() }
        guard payload.pathExtension == ShareInbox.linkExtension else {
            // 收件箱里的文件是分享扩展从 provider 的临时副本拷来的, 修改时间只是拷贝的那一刻。
            return await ingestor.ingest(urls: [payload], move: true, dating: .none, allowing: [.image, .pdf])
        }
        guard let text = try? String(contentsOf: payload, encoding: .utf8),
              let link = URL(string: text.trimmingCharacters(in: .whitespacesAndNewlines))
        else { return IngestResult(ignored: 1) }
        return await ingestor.ingest(webLink: link, allowing: [.image, .pdf])
    }
}