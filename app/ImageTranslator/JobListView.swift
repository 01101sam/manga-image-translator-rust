import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct JobListView: View {
    @EnvironmentObject private var session: AppSession
    @State private var photoItems: [PhotosPickerItem] = []
    @State private var importing = false
    @State private var previewJobId: String?
    @State private var selectedJobId: String?

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if TestHooks.fromProcessInfo().isolatesSession {
                    Text(ProcessInfo.processInfo.arguments.joined(separator: " "))
                        .frame(width: 1, height: 1)
                        .accessibilityIdentifier("launch-args")
                }
                if let banner = session.banner {
                    Text(banner)
                        .font(.footnote)
                        .foregroundStyle(.white)
                        .frame(maxWidth: .infinity)
                        .padding(8)
                        .background(Color.orange)
                        .accessibilityIdentifier("engine-banner")
                }
                List {
                    ForEach(session.jobs, id: \.snapshot.jobId) { row in
                        JobRowView(row: row) {
                            Task { await session.requestCancel(row.snapshot.jobId) }
                        }
                        .listRowBackground(
                            selectedJobId == row.snapshot.jobId
                                ? Color.accentColor.opacity(0.16)
                                : Color.clear
                        )
                        .contentShape(Rectangle())
                        .onTapGesture {
                            selectedJobId = row.snapshot.jobId
                            if canOpenJobPreview(row.snapshot.state) {
                                previewJobId = row.snapshot.jobId
                            }
                        }
                    }
                    .onDelete { session.removeJobs(at: $0) }
                    .onMove { session.moveJobs(from: $0, to: $1) }
                }
                .overlay {
                    if session.jobs.isEmpty {
                        ContentUnavailableView("还没有 Job", systemImage: "photo", description: Text("从相册、文件或文件夹导入图片。"))
                    }
                    JobListKeyCatcher(
                        onMove: { moveSelection(by: $0) },
                        onEnter: { openSelectedPreview() }
                    )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .allowsHitTesting(false)
                }
            }
            .navigationTitle("任务")
            .onAppear { session.consumePendingHooks() }
            .onChange(of: session.jobs.map(\.snapshot.jobId)) { _, ids in
                if let selectedJobId, !ids.contains(selectedJobId) {
                    self.selectedJobId = nil
                }
                if let previewJobId, !ids.contains(previewJobId) {
                    self.previewJobId = nil
                }
            }
            .toolbar {
                ToolbarItemGroup(placement: .primaryAction) {
                    EditButton()
                    PhotosPicker(selection: $photoItems, maxSelectionCount: 32, matching: .images) {
                        Label("相册", systemImage: "photo.on.rectangle")
                    }
                    Button {
                        importing = true
                    } label: {
                        Label("文件", systemImage: "folder")
                    }
                    Button("更换 Daemon") { session.forgetPairing() }
                }
            }
            .fileImporter(
                isPresented: $importing,
                allowedContentTypes: [.image, .folder],
                allowsMultipleSelection: true
            ) { result in
                if case .success(let urls) = result {
                    Task { await session.importURLs(urls) }
                }
            }
            .onChange(of: photoItems) { _, items in
                Task {
                    for item in items {
                        if let data = try? await item.loadTransferable(type: Data.self) {
                            var label = imageUploadLabel(for: data)
                            if label.mime == "application/octet-stream", let type = item.supportedContentTypes.first {
                                label = ImageUploadLabel(
                                    filename: "photo.\(type.preferredFilenameExtension ?? "bin")",
                                    mime: type.preferredMIMEType ?? "application/octet-stream"
                                )
                            }
                            await session.submitImage(data, filename: label.filename, mime: label.mime)
                        }
                    }
                    photoItems = []
                }
            }
            .sheet(isPresented: Binding(
                get: { previewJobId != nil },
                set: { if !$0 { previewJobId = nil } }
            )) {
                ArtifactPreview(
                    jobId: previewJobId ?? "",
                    onMoveDone: { delta in
                        if let next = adjacentDoneJobId(jobs: session.jobs, currentId: previewJobId, delta: delta) {
                            previewJobId = next
                            selectedJobId = next
                        }
                    }
                )
                .environmentObject(session)
            }
        }
    }

    private func moveSelection(by delta: Int) {
        if previewJobId != nil {
            if let next = adjacentDoneJobId(jobs: session.jobs, currentId: previewJobId, delta: delta) {
                previewJobId = next
                selectedJobId = next
            }
            return
        }
        let ids = session.jobs.map(\.snapshot.jobId)
        let current = selectedJobId.flatMap { ids.firstIndex(of: $0) }
        if let idx = moveJobHighlight(count: ids.count, current: current, delta: delta) {
            selectedJobId = ids[idx]
        }
    }

    private func openSelectedPreview() {
        guard let selectedJobId,
              let row = session.jobs.first(where: { $0.snapshot.jobId == selectedJobId }),
              canOpenJobPreview(row.snapshot.state)
        else { return }
        previewJobId = selectedJobId
    }
}

struct JobRowView: View {
    let row: JobRow
    let onCancel: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            thumbnail
            VStack(alignment: .leading, spacing: 4) {
                Text(shortJobId(row.snapshot.jobId))
                    .font(.caption.monospaced())
                    .lineLimit(1)
                Text(label(for: row.snapshot))
                    .font(.subheadline)
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("job-row")
            .accessibilityValue(label(for: row.snapshot))
            .accessibilityLabel(label(for: row.snapshot))
            Spacer()
            if canCancelJob(row.snapshot.state) {
                Button("取消", action: onCancel)
                    .disabled(row.cancelPending)
                    .accessibilityIdentifier("job-cancel")
            }
        }
    }

    @ViewBuilder
    private var thumbnail: some View {
        if let data = row.thumbnail, let image = UIImage(data: data) {
            Image(uiImage: image)
                .resizable()
                .scaledToFill()
                .frame(width: 44, height: 44)
                .clipped()
                .cornerRadius(6)
        } else {
            Image(systemName: "doc")
                .font(.title3)
                .foregroundStyle(.secondary)
                .frame(width: 44, height: 44)
        }
    }

    private func label(for snap: JobSnapshot) -> String {
        switch snap.state {
        case .queued:
            if let pos = snap.positionInQueue {
                return "排队中 · 第 \(pos) 位"
            }
            return "排队中"
        case .running:
            return "翻译中"
        case .done:
            return "已完成"
        case .failed:
            return snap.error.map { "失败 · \($0)" } ?? "失败"
        case .cancelled:
            return "已取消"
        }
    }
}

struct ArtifactPreview: View {
    @EnvironmentObject private var session: AppSession
    let jobId: String
    var onMoveDone: (Int) -> Void = { _ in }

    var body: some View {
        NavigationStack {
            Group {
                if let data = session.artifacts[jobId], let image = PlatformImage(data: data) {
                    ScrollView {
                        Image(platformImage: image)
                            .resizable()
                            .scaledToFit()
                    }
                } else {
                    ProgressView("正在读取 Artifact…")
                }
            }
            .overlay {
                JobListKeyCatcher(
                    onMove: onMoveDone,
                    onEnter: {}
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .allowsHitTesting(false)
            }
            .navigationTitle("译文")
            .toolbar {
                if let data = session.artifacts[jobId] {
                    ShareLink(item: ArtifactFile(data: data, name: "\(jobId).png"), preview: SharePreview("译文"))
                }
            }
        }
    }
}

struct JobListKeyCatcher: UIViewControllerRepresentable {
    var onMove: (Int) -> Void
    var onEnter: () -> Void

    func makeUIViewController(context: Context) -> JobListKeyController {
        let controller = JobListKeyController()
        controller.onMove = onMove
        controller.onEnter = onEnter
        return controller
    }

    func updateUIViewController(_ controller: JobListKeyController, context: Context) {
        controller.onMove = onMove
        controller.onEnter = onEnter
        controller.becomeFirstResponder()
    }
}

final class JobListKeyController: UIViewController {
    var onMove: ((Int) -> Void)?
    var onEnter: (() -> Void)?

    override var canBecomeFirstResponder: Bool { true }

    override var keyCommands: [UIKeyCommand]? {
        [
            UIKeyCommand(input: UIKeyCommand.inputUpArrow, modifierFlags: [], action: #selector(up)),
            UIKeyCommand(input: UIKeyCommand.inputDownArrow, modifierFlags: [], action: #selector(down)),
            UIKeyCommand(input: "\r", modifierFlags: [], action: #selector(enter)),
            UIKeyCommand(input: "\n", modifierFlags: [], action: #selector(enter)),
        ]
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        becomeFirstResponder()
    }

    @objc private func up() { onMove?(-1) }
    @objc private func down() { onMove?(1) }
    @objc private func enter() { onEnter?() }
}

struct ArtifactFile: Transferable {
    let data: Data
    let name: String

    static var transferRepresentation: some TransferRepresentation {
        DataRepresentation(exportedContentType: .png) { file in file.data }
            .suggestedFileName("translated.png")
    }
}
