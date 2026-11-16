import UIKit

/// 分享面板里的「图片翻译」: 把图片、PDF、图片链接放进 App Group 的收件箱, 显示一句确认就结束。
/// 翻译在 App 里开始; 扩展不连 Daemon, 也不解码图片。
final class ShareViewController: UIViewController {
    private let spinner = UIActivityIndicatorView(style: .large)
    private let label = UILabel()
    private let dismissButton = UIButton(type: .system)
    private var started = false
    private var succeeded = false

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        label.numberOfLines = 0
        label.textAlignment = .center
        label.font = .preferredFont(forTextStyle: .headline)
        label.accessibilityIdentifier = "share-message"
        spinner.startAnimating()
        var configuration = UIButton.Configuration.filled()
        configuration.title = "好"
        dismissButton.configuration = configuration
        dismissButton.accessibilityIdentifier = "share-dismiss"
        dismissButton.isHidden = true
        dismissButton.addAction(UIAction { [weak self] _ in self?.close() }, for: .touchUpInside)
        let stack = UIStackView(arrangedSubviews: [spinner, label, dismissButton])
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
            finish("没有共享容器, 无法加入。请重新安装 App。", success: false, stays: true)
            return
        }
        let items = (extensionContext?.inputItems as? [NSExtensionItem]) ?? []
        let result = await ShareDepositor(inbox: inbox).deposit(items: items)
        let reasons = result.skips.reasons.map { "· \($0)" }.joined(separator: "\n")
        if result.added == 0 {
            finish("没有可以加入的图片或 PDF" + (reasons.isEmpty ? "" : "\n\(reasons)"), success: false, stays: !reasons.isEmpty)
        } else if result.skipped > 0 {
            finish("已加入 \(result.added) 项，打开 App 后开始翻译\n另有 \(result.skipped) 项没能加入:\n\(reasons)", success: true, stays: true)
        } else {
            finish("已加入，打开 App 后开始翻译", success: true, stays: false)
        }
    }

    /// 有东西没能加入时面板一直留着, 等用户点「好」: 1.2 秒一闪而过的话, 用户只看到「已加入」, 少了几张也不知道。
    private func finish(_ message: String, success: Bool, stays: Bool) {
        spinner.stopAnimating()
        label.text = message
        succeeded = success
        if stays {
            dismissButton.isHidden = false
            return
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + (success ? 1.2 : 2.0)) { [weak self] in self?.close() }
    }

    private func close() {
        if succeeded {
            extensionContext?.completeRequest(returningItems: nil)
        } else {
            extensionContext?.cancelRequest(withError: NSError(domain: "ShareExtension", code: 1))
        }
    }
}