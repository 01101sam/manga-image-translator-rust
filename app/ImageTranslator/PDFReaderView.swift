import PDFKit
import Photos
import SwiftUI
import UIKit
import UniformTypeIdentifiers

@MainActor
final class PDFSession: ObservableObject {
    @Published var document: PDFDocument?
    @Published var pages: [ReaderPage] = []
    @Published var currentIndex: Int = 0
    @Published var sourceURL: URL?
    @Published var exportURL: URL?

    func load(_ url: URL) {
        let accessed = url.startAccessingSecurityScopedResource()
        defer {
            if accessed { url.stopAccessingSecurityScopedResource() }
        }
        guard let data = try? Data(contentsOf: url), let doc = PDFDocument(data: data) else {
            document = nil
            pages = []
            sourceURL = nil
            return
        }
        document = doc
        sourceURL = url
        currentIndex = 0
        pages = (0..<doc.pageCount).map { ReaderPage(index: $0, translation: .untranslated, viewing: .original, jobId: nil) }
    }

    func apply(_ event: PageEvent, at index: Int) -> PageCommand {
        guard pages.indices.contains(index) else { return .none }
        let (next, command) = reducePage(pages[index], event)
        pages[index] = next
        return command
    }

    func move(by delta: Int) {
        guard !pages.isEmpty else { return }
        currentIndex = min(max(currentIndex + delta, 0), pages.count - 1)
    }

    func rasterizeCurrent() -> Data? {
        rasterize(at: currentIndex)
    }

    func rasterize(at index: Int) -> Data? {
        guard let page = document?.page(at: index) else { return nil }
        let box = page.bounds(for: .mediaBox)
        let scale = 300.0 / 72.0
        let size = CGSize(width: max(box.width * scale, 1), height: max(box.height * scale, 1))
        let renderer = UIGraphicsImageRenderer(size: size)
        let image = renderer.image { ctx in
            UIColor.white.setFill()
            ctx.fill(CGRect(origin: .zero, size: size))
            ctx.cgContext.saveGState()
            ctx.cgContext.translateBy(x: 0, y: size.height)
            ctx.cgContext.scaleBy(x: scale, y: -scale)
            page.draw(with: .mediaBox, to: ctx.cgContext)
            ctx.cgContext.restoreGState()
        }
        return image.pngData()
    }

    func displayImage(artifacts: [String: Data]) -> UIImage? {
        guard pages.indices.contains(currentIndex) else { return nil }
        let page = pages[currentIndex]
        if page.translation == .done, page.viewing == .translated,
           let jobId = page.jobId, let data = artifacts[jobId], let image = UIImage(data: data)
        {
            return image
        }
        return rasterizePreview(at: currentIndex)
    }

    func assembleExport(artifacts: [String: Data]) -> URL? {
        guard let document else { return nil }
        let out = PDFDocument()
        for i in 0..<document.pageCount {
            if let jobId = pages[safe: i]?.jobId,
               pages[i].translation == .done,
               let data = artifacts[jobId],
               let image = UIImage(data: data),
               let pdfPage = PDFPage(image: image)
            {
                out.insert(pdfPage, at: out.pageCount)
            } else if let original = document.page(at: i) {
                out.insert(original, at: out.pageCount)
            }
        }
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("translated.pdf")
        out.write(to: url)
        exportURL = url
        return url
    }

    private func rasterizePreview(at index: Int) -> UIImage? {
        guard let page = document?.page(at: index) else { return nil }
        let box = page.bounds(for: .mediaBox)
        let scale = 2.0
        let size = CGSize(width: max(box.width * scale, 1), height: max(box.height * scale, 1))
        let renderer = UIGraphicsImageRenderer(size: size)
        return renderer.image { ctx in
            UIColor.white.setFill()
            ctx.fill(CGRect(origin: .zero, size: size))
            ctx.cgContext.saveGState()
            ctx.cgContext.translateBy(x: 0, y: size.height)
            ctx.cgContext.scaleBy(x: scale, y: -scale)
            page.draw(with: .mediaBox, to: ctx.cgContext)
            ctx.cgContext.restoreGState()
        }
    }
}

private extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}

struct PDFReaderView: View {
    @EnvironmentObject private var session: AppSession
    @StateObject private var reader = PDFSession()
    @State private var importing = false

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if let image = reader.displayImage(artifacts: session.artifacts) {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFit()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .background(ReaderKeyCatcher(
                            onMove: { reader.move(by: $0) },
                            onSpace: { handleSpace() }
                        ))
                } else {
                    ContentUnavailableView("打开 PDF", systemImage: "doc", description: Text("原文在本地渲染，Daemon 不会收到这份 PDF。"))
                }
                if !reader.pages.isEmpty {
                    statusBar
                }
            }
            .navigationTitle("阅读")
            .toolbar {
                ToolbarItemGroup(placement: .primaryAction) {
                    Button("打开") { importing = true }
                    if !reader.pages.isEmpty {
                        Button("翻译全部") { Task { await translateAll() } }
                        if let url = reader.exportURL ?? reader.assembleExport(artifacts: session.artifacts) {
                            ShareLink(item: url) { Text("导出") }
                        }
                        Button("存图") { saveCurrentToPhotos() }
                    }
                }
            }
            .fileImporter(isPresented: $importing, allowedContentTypes: [.pdf], allowsMultipleSelection: false) { result in
                if case .success(let urls) = result, let url = urls.first {
                    reader.load(url)
                }
            }
            .onChange(of: session.jobs) { _, jobs in
                syncJobs(jobs)
            }
        }
    }

    private var statusBar: some View {
        let page = reader.pages[reader.currentIndex]
        return HStack {
            Button("上一页") { reader.move(by: -1) }
            Spacer()
            Text("第 \(reader.currentIndex + 1)/\(reader.pages.count) 页 · \(pageLabel(page))")
                .font(.footnote)
            Spacer()
            Button("下一页") { reader.move(by: 1) }
        }
        .padding()
    }

    private func pageLabel(_ page: ReaderPage) -> String {
        switch page.translation {
        case .untranslated: return "未翻译"
        case .queued: return "排队中"
        case .running: return "翻译中"
        case .done: return page.viewing == .translated ? "译文" : "原文"
        }
    }

    private func handleSpace() {
        let command = reader.apply(.space, at: reader.currentIndex)
        if command == .enqueue {
            Task { await enqueue(at: reader.currentIndex) }
        }
    }

    private func translateAll() async {
        for index in untranslatedIndexes(reader.pages) {
            await enqueue(at: index)
        }
    }

    private func enqueue(at index: Int) async {
        guard let data = reader.rasterize(at: index) else { return }
        let before = Set(session.jobs.map(\.snapshot.jobId))
        await session.submitImage(data, filename: "page-\(index + 1).png", mime: "image/png")
        if let jobId = session.jobs.map(\.snapshot.jobId).first(where: { !before.contains($0) }) {
            _ = reader.apply(.submitted(jobId: jobId), at: index)
        }
    }

    private func syncJobs(_ jobs: [JobRow]) {
        for (idx, page) in reader.pages.enumerated() {
            guard let jobId = page.jobId, let row = jobs.first(where: { $0.snapshot.jobId == jobId }) else { continue }
            _ = reader.apply(.jobState(row.snapshot.state), at: idx)
        }
    }

    private func saveCurrentToPhotos() {
        guard let image = reader.displayImage(artifacts: session.artifacts) else { return }
        PHPhotoLibrary.requestAuthorization(for: .addOnly) { status in
            guard status == .authorized || status == .limited else { return }
            UIImageWriteToSavedPhotosAlbum(image, nil, nil, nil)
        }
    }
}

struct ReaderKeyCatcher: UIViewControllerRepresentable {
    var onMove: (Int) -> Void
    var onSpace: () -> Void

    func makeUIViewController(context: Context) -> ReaderKeyController {
        let controller = ReaderKeyController()
        controller.onMove = onMove
        controller.onSpace = onSpace
        return controller
    }

    func updateUIViewController(_ controller: ReaderKeyController, context: Context) {
        controller.onMove = onMove
        controller.onSpace = onSpace
        controller.becomeFirstResponder()
    }
}

final class ReaderKeyController: UIViewController {
    var onMove: ((Int) -> Void)?
    var onSpace: (() -> Void)?

    override var canBecomeFirstResponder: Bool { true }

    override var keyCommands: [UIKeyCommand]? {
        [
            UIKeyCommand(input: UIKeyCommand.inputUpArrow, modifierFlags: [], action: #selector(up)),
            UIKeyCommand(input: UIKeyCommand.inputDownArrow, modifierFlags: [], action: #selector(down)),
            UIKeyCommand(input: UIKeyCommand.inputLeftArrow, modifierFlags: [], action: #selector(left)),
            UIKeyCommand(input: UIKeyCommand.inputRightArrow, modifierFlags: [], action: #selector(right)),
            UIKeyCommand(input: " ", modifierFlags: [], action: #selector(space)),
        ]
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        becomeFirstResponder()
    }

    @objc private func up() { onMove?(-1) }
    @objc private func down() { onMove?(1) }
    @objc private func left() { onMove?(-1) }
    @objc private func right() { onMove?(1) }
    @objc private func space() { onSpace?() }
}
