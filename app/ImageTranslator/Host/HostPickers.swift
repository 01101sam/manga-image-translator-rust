import PhotosUI
import UIKit
import UniformTypeIdentifiers

/// 图片的三个来源。iPad 与 visionOS 的 action sheet 必须有 popover 锚点。
enum ImageSource: CaseIterable {
    case photos
    case files
    case folder

    var title: String {
        switch self {
        case .photos: return "照片"
        case .files: return "文件"
        case .folder: return "文件夹"
        }
    }
}

enum PickerOutcome {
    case providers([NSItemProvider])
    case urls([URL])
    case cancelled
}

/// 在宿主的 view controller 上弹系统选择器, 把委托回调变成 async。同一时间只允许一个选择器。
@MainActor
final class PickerPresenter: NSObject, PHPickerViewControllerDelegate, UIDocumentPickerDelegate, UIAdaptivePresentationControllerDelegate {
    private weak var host: UIViewController?
    private var pending: CheckedContinuation<PickerOutcome, Never>?

    init(host: UIViewController) {
        self.host = host
    }

    var isBusy: Bool { pending != nil }

    func chooseImageSource() async -> ImageSource? {
        guard let host, !isBusy else { return nil }
        return await withCheckedContinuation { continuation in
            let sheet = UIAlertController(title: nil, message: nil, preferredStyle: .actionSheet)
            for source in ImageSource.allCases {
                sheet.addAction(UIAlertAction(title: source.title, style: .default) { _ in continuation.resume(returning: source) })
            }
            sheet.addAction(UIAlertAction(title: "取消", style: .cancel) { _ in continuation.resume(returning: nil) })
            if let popover = sheet.popoverPresentationController {
                popover.sourceView = host.view
                popover.sourceRect = CGRect(x: host.view.bounds.midX, y: host.view.bounds.midY, width: 0, height: 0)
                popover.permittedArrowDirections = []
            }
            host.present(sheet, animated: true)
        }
    }

    func pickPhotos(limit: Int) async -> PickerOutcome {
        var configuration = PHPickerConfiguration()
        configuration.filter = .images
        configuration.selectionLimit = max(0, limit)
        // HEIC 等相册原格式 Daemon 不认, 让系统转成 JPEG / PNG 再给文件。
        configuration.preferredAssetRepresentationMode = .compatible
        let picker = PHPickerViewController(configuration: configuration)
        picker.delegate = self
        return await present(picker)
    }

    func pickDocuments(types: [UTType], multiple: Bool) async -> PickerOutcome {
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: types, asCopy: false)
        picker.allowsMultipleSelection = multiple
        picker.delegate = self
        return await present(picker)
    }

    private func present(_ controller: UIViewController) async -> PickerOutcome {
        guard let host, !isBusy else { return .cancelled }
        return await withCheckedContinuation { continuation in
            pending = continuation
            controller.presentationController?.delegate = self
            host.present(controller, animated: true)
        }
    }

    private func finish(_ outcome: PickerOutcome) {
        guard let continuation = pending else { return }
        pending = nil
        continuation.resume(returning: outcome)
    }

    nonisolated func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        let providers = results.map(\.itemProvider)
        Task { @MainActor in
            picker.dismiss(animated: true)
            finish(providers.isEmpty ? .cancelled : .providers(providers))
        }
    }

    nonisolated func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        Task { @MainActor in finish(urls.isEmpty ? .cancelled : .urls(urls)) }
    }

    nonisolated func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        Task { @MainActor in finish(.cancelled) }
    }

    nonisolated func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        Task { @MainActor in finish(.cancelled) }
    }
}
