import SwiftUI
import WebKit

struct ConfigWebView: View {
    @EnvironmentObject private var session: AppSession

    var body: some View {
        NavigationStack {
            Group {
                if let endpoint = session.pairing.endpoint {
                    DaemonWebView(url: endpoint.baseURL)
                } else {
                    ContentUnavailableView("尚未配对", systemImage: "slider.horizontal.3")
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
