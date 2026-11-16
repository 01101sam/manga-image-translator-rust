import Foundation

/// 「打开方式」交来的文件 URL 的中转: onOpenURL 在 SwiftUI 一侧, 导入在 NshHostViewController 一侧。
/// 界面还没起来 (冷启动) 就先攒着; 多选文件用「打开方式」时系统会一个接一个地交, 所以隔一小会儿才合成一批发出,
/// 一批里的 PDF 才能「只开第一个」。
@MainActor
final class OpenURLRouter {
    static let batchDelay: UInt64 = 150_000_000

    private var pending: [URL] = []
    private var flushing: Task<Void, Never>?

    var handler: (([URL]) -> Void)? {
        didSet { scheduleFlush() }
    }

    func open(_ url: URL) {
        guard url.isFileURL else { return }
        pending.append(url)
        scheduleFlush()
    }

    private func scheduleFlush() {
        guard handler != nil, !pending.isEmpty else { return }
        flushing?.cancel()
        flushing = Task { [weak self] in
            try? await Task.sleep(nanoseconds: Self.batchDelay)
            guard !Task.isCancelled, let self, let handler else { return }
            let batch = pending
            pending = []
            handler(batch)
        }
    }
}
