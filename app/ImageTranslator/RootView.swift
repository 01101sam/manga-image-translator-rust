import SwiftUI

struct RootView: View {
    @EnvironmentObject private var session: AppSession

    var body: some View {
        Group {
            if session.isPaired {
                TabView {
                    JobListView()
                        .tabItem { Label("任务", systemImage: "list.bullet") }
                    PDFReaderView()
                        .tabItem { Label("阅读", systemImage: "doc.richtext") }
                    ConfigWebView()
                        .tabItem { Label("配置", systemImage: "slider.horizontal.3") }
                }
            } else {
                DiscoveryView()
            }
        }
    }
}

struct DiscoveryView: View {
    @EnvironmentObject private var session: AppSession

    var body: some View {
        NavigationStack {
            VStack(spacing: 16) {
                switch session.pairing {
                case .browsing:
                    browseList
                case .requesting(let endpoint):
                    ProgressView("正在向 \(endpoint.name) 请求配对码…")
                case .awaitingCode(let endpoint), .confirming(let endpoint, _):
                    PairingCodeForm(daemonName: endpoint.name)
                case .failed(let endpoint, let message):
                    VStack(spacing: 12) {
                        Text(endpoint.name).font(.headline)
                        Text(message).foregroundStyle(.red)
                        Button("重试") { session.retryPairing() }
                    }
                case .unauthorized(let endpoint):
                    VStack(spacing: 12) {
                        Text("与 \(endpoint.name) 的配对已失效")
                        Text("请重新配对。")
                        Button("重新配对") { session.retryPairing() }
                    }
                case .paired:
                    EmptyView()
                }
            }
            .padding()
            .navigationTitle("图片翻译")
        }
    }

    private var browseList: some View {
        Group {
            if session.browser.daemons.isEmpty {
                ContentUnavailableView(
                    "正在搜索 Daemon",
                    systemImage: "bonjour",
                    description: Text("请确认本机已启动 Daemon。")
                )
            } else {
                List(session.browser.daemons, id: \.self) { endpoint in
                    Button {
                        session.select(endpoint)
                    } label: {
                        VStack(alignment: .leading) {
                            Text(endpoint.name).font(.headline)
                            Text("\(endpoint.host):\(endpoint.port)")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
    }
}

struct PairingCodeForm: View {
    @EnvironmentObject private var session: AppSession
    let daemonName: String

    var body: some View {
        VStack(spacing: 12) {
            Text("请输入 \(daemonName) 控制台中的 6 位配对码")
                .multilineTextAlignment(.center)
            TextField("000000", text: $session.pairingCode)
                .keyboardType(.numberPad)
                .textContentType(.oneTimeCode)
                .multilineTextAlignment(.center)
                .font(.largeTitle.monospacedDigit())
                .onChange(of: session.pairingCode) { _, value in
                    let digits = value.filter(\.isNumber)
                    if digits != value || digits.count > 6 {
                        session.pairingCode = String(digits.prefix(6))
                    }
                }
            Button("确认") {
                session.submitPairingCode()
            }
            .disabled(session.pairingCode.count != 6)
            Button("返回") { session.forgetPairing() }
                .foregroundStyle(.secondary)
        }
    }
}
