import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

struct JobListView: View {
    @EnvironmentObject private var session: AppSession
    @State private var photoItems: [PhotosPickerItem] = []
    @State private var importing = false
    @State private var previewJobId: String?

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                Text(ProcessInfo.processInfo.arguments.joined(separator: " "))
                    .frame(width: 1, height: 1)
                    .accessibilityIdentifier("launch-args")
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
                        .contentShape(Rectangle())
                        .onTapGesture {
                            if row.snapshot.state == .done {
                                previewJobId = row.snapshot.jobId
                            }
                        }
                    }
                }
                .overlay {
                    if session.jobs.isEmpty {
                        ContentUnavailableView("还没有 Job", systemImage: "photo", description: Text("从相册、文件或文件夹导入图片。"))
                    }
                }
            }
            .navigationTitle("任务")
            .onAppear { session.consumePendingImageIfNeeded() }
            .toolbar {
                ToolbarItemGroup(placement: .primaryAction) {
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
                            await session.submitImage(data, filename: "photo.jpg", mime: "image/jpeg")
                        }
                    }
                    photoItems = []
                }
            }
            .dropDestination(for: URL.self) { urls, _ in
                Task { await session.importURLs(urls) }
                return true
            }
            .sheet(item: Binding(
                get: { previewJobId.map(PreviewItem.init(id:)) },
                set: { previewJobId = $0?.id }
            )) { item in
                ArtifactPreview(jobId: item.id)
                    .environmentObject(session)
            }
        }
    }
}

private struct PreviewItem: Identifiable {
    var id: String
}

struct JobRowView: View {
    let row: JobRow
    let onCancel: () -> Void

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                Text(row.snapshot.jobId)
                    .font(.caption.monospaced())
                    .lineLimit(1)
                Text(label(for: row.snapshot))
                    .font(.subheadline)
                    .accessibilityIdentifier("job-state")
            }
            Spacer()
            if canCancelJob(row.snapshot.state) {
                Button("取消", action: onCancel)
                    .disabled(row.cancelPending)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("job-row")
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
            .navigationTitle("译文")
            .toolbar {
                if let data = session.artifacts[jobId] {
                    ShareLink(item: ArtifactFile(data: data, name: "\(jobId).png"), preview: SharePreview("译文"))
                }
            }
        }
    }
}

struct ArtifactFile: Transferable {
    let data: Data
    let name: String

    static var transferRepresentation: some TransferRepresentation {
        DataRepresentation(exportedContentType: .png) { file in file.data }
            .suggestedFileName("translated.png")
    }
}
