import Foundation
import UIKit

/// artifact.save: 宿主自己下载 Artifact 到临时文件, 再弹系统分享面板。
enum ArtifactSaver {
    /// 按应答的 Content-Type 补扩展名。HOST_PROTOCOL.md 只列了 png 和 html, 原图常是 jpeg / webp, 一起认。
    static func fileExtension(forContentType contentType: String?) -> String {
        let type = (contentType ?? "").split(separator: ";").first.map { $0.trimmingCharacters(in: .whitespaces).lowercased() } ?? ""
        switch type {
        case "image/png": return "png"
        case "image/jpeg": return "jpg"
        case "image/webp": return "webp"
        case "text/html": return "html"
        default: return "bin"
        }
    }

    static func download(from url: URL, filename: String, files: ManagedFiles) async throws -> URL {
        guard url.scheme == "http" || url.scheme == "https" else { throw HostCallError("保存地址不是 http 地址") }
        let (temporary, response): (URL, URLResponse)
        do {
            (temporary, response) = try await URLSession.shared.download(from: url)
        } catch {
            throw HostCallError("下载失败: \(error.localizedDescription)")
        }
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
            try? FileManager.default.removeItem(at: temporary)
            throw HostCallError("下载失败: HTTP \((response as? HTTPURLResponse)?.statusCode ?? 0)")
        }
        let name = ManagedFiles.safeFileName(filename, fallback: "image")
        let target = files.scratchFile(named: "\(name).\(fileExtension(forContentType: http.value(forHTTPHeaderField: "Content-Type")))")
        try FileManager.default.moveItem(at: temporary, to: target)
        return target
    }

    @MainActor
    static func present(_ file: URL, from host: UIViewController) {
        let sheet = UIActivityViewController(activityItems: [file], applicationActivities: nil)
        sheet.completionWithItemsHandler = { _, _, _, _ in try? FileManager.default.removeItem(at: file) }
        if let popover = sheet.popoverPresentationController {
            popover.sourceView = host.view
            popover.sourceRect = CGRect(x: host.view.bounds.midX, y: host.view.bounds.midY, width: 0, height: 0)
            popover.permittedArrowDirections = []
        }
        host.present(sheet, animated: true)
    }
}
