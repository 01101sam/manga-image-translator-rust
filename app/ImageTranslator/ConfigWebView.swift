import SwiftUI
import WebKit

struct ConfigWebView: View {
    @EnvironmentObject private var session: AppSession
    #if os(visionOS)
    @EnvironmentObject private var windowSettings: WindowSettings
    #endif

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                #if os(visionOS)
                Picker("窗口方向", selection: $windowSettings.windowLandscape) {
                    Text("竖屏").tag(false)
                    Text("横屏").tag(true)
                }
                .pickerStyle(.segmented)
                .padding()
                .accessibilityIdentifier("window-orientation")
                #endif
                Group {
                    if let endpoint = session.pairing.endpoint {
                        DaemonWebView(url: endpoint.baseURL)
                    } else {
                        ContentUnavailableView("尚未配对", systemImage: "slider.horizontal.3")
                    }
                }
            }
            .navigationTitle("配置")
            .toolbar {
                if session.engine == .stopped {
                    Text("Engine 已停止")
                        .foregroundStyle(.orange)
                }
            }
        }
    }
}

struct DaemonWebView: UIViewRepresentable {
    let url: URL

    func makeUIView(context: Context) -> WKWebView {
        let view = WKWebView()
        view.accessibilityIdentifier = "config-webview"
        view.isAccessibilityElement = true
        view.load(URLRequest(url: url))
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {
        if view.url?.host != url.host || view.url?.port != url.port {
            view.load(URLRequest(url: url))
        }
    }
}
