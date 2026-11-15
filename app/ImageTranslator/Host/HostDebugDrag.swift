import UIKit

/// 测试钩子用的 NSItemProvider: 进程内创建的 provider 把 suggestedName 填成类型描述 ("PNG图像"), 真实的拖放来源给的是文件名, 这里补成一样。
enum DebugProvider {
    static func make(_ url: URL) -> NSItemProvider? {
        let provider = NSItemProvider(contentsOf: url)
        provider?.suggestedName = url.deletingPathExtension().lastPathComponent
        return provider
    }
}

/// 测试钩子 -debug-drag-source <目录>: 窗口左下角放一个可拖的小方块, 里面是目录里前几个文件。
/// 系统拖放没法从命令行驱动, UI 测试用它在 App 内部发起一次真正的拖放, 落进 UIDropInteraction。
/// 文件先克隆一份再包成 NSItemProvider: 拖放的读取会把文件搬走, 不能动原目录。
final class DebugDragSource: NSObject, UIDragInteractionDelegate {
    let view = UIView()
    private let files: [URL]

    init(folder: URL, count: Int, scratch: URL) {
        try? FileManager.default.createDirectory(at: scratch, withIntermediateDirectories: true)
        files = FileIngestor.files(inDirectory: folder, allowing: [.image, .pdf]).prefix(count).compactMap { source in
            let target = scratch.appendingPathComponent(source.lastPathComponent)
            return (try? FileManager.default.copyItem(at: source, to: target)) == nil ? nil : target
        }
        super.init()
        view.backgroundColor = .systemOrange
        view.layer.cornerRadius = 12
        view.isAccessibilityElement = true
        view.accessibilityIdentifier = "debug-drag-source"
        view.addInteraction(UIDragInteraction(delegate: self))
    }

    func dragInteraction(_ interaction: UIDragInteraction, itemsForBeginning session: UIDragSession) -> [UIDragItem] {
        files.compactMap(DebugProvider.make).map { UIDragItem(itemProvider: $0) }
    }

    func install(in host: UIView) {
        view.translatesAutoresizingMaskIntoConstraints = false
        host.addSubview(view)
        NSLayoutConstraint.activate([
            view.widthAnchor.constraint(equalToConstant: 72),
            view.heightAnchor.constraint(equalToConstant: 72),
            view.leadingAnchor.constraint(equalTo: host.safeAreaLayoutGuide.leadingAnchor, constant: 16),
            view.bottomAnchor.constraint(equalTo: host.safeAreaLayoutGuide.bottomAnchor, constant: -96),
        ])
    }
}
