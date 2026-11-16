import UIKit

/// 分享面板里的「图片翻译」: 把图片、PDF、图片链接放进 App Group 的收件箱, 显示一句确认就结束。
/// 翻译在 App 里开始; 扩展不连 Daemon, 也不解码图片。
final class ShareViewController: UIViewController {
    private let spinner = UIActivityIndicatorView(style: .large)
    private let label = UILabel()
    private var started = false

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        label.numberOfLines = 0
        label.textAlignment = .center
        label.font = .preferredFont(forTextStyle: .headline)
        label.accessibilityIdentifier = "share-message"
        spinner.startAnimating()
        let stack = UIStackView(arrangedSubviews: [spinner, label])
        stack.axis = .vertical
        stack.spacing = 16
        stack.alignment = .center
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            stack.leadingAnchor.constraint(greaterThanOrEqualTo: view.leadingAnchor, constant: 24),
            stack.trailingAnchor.constraint(lessThanOrEqualTo: view.trailingAnchor, constant: -24),
        ])
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard !started else { return }
        started = true
        Task { await run() }
    }

    private func run() async {
        // 取不到容器就如实说, 不能让用户以为已经加入。
        guard let inbox = ShareInbox.directory() else {
            finish("没有共享容器, 无法加入。请重新安装 App。", success: false)
            return
        }
        let items = (extensionContext?.inputItems as? [NSExtensionItem]) ?? []
        let result = await ShareDepositor(inbox: inbox).deposit(items: items)
        if result.added == 0 {
            finish("没有可以加入的图片或 PDF", success: false)
        } else if result.skipped > 0 {
            finish("已加入 \(result.added) 项，打开 App 后开始翻译\n另有 \(result.skipped) 项无法加入", success: true)
        } else {
            finish("已加入，打开 App 后开始翻译", success: true)
        }
    }

    private func finish(_ message: String, success: Bool) {
        spinner.stopAnimating()
        label.text = message
        let delay: TimeInterval = success ? 1.2 : 2.0
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
            if success {
                self?.extensionContext?.completeRequest(returningItems: nil)
            } else {
                self?.extensionContext?.cancelRequest(withError: NSError(domain: "ShareExtension", code: 1))
            }
        }
    }
}
