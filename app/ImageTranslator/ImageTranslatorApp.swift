import SwiftUI

@main
struct ImageTranslatorApp: App {
    @StateObject private var windowSettings = WindowSettings()

    var body: some Scene {
        WindowGroup {
            HostRootView()
                .environmentObject(windowSettings)
                .modifier(AppWindowFrame(settings: windowSettings))
        }
        .windowResizability(.contentSize)
        .defaultSize(windowSettings.idealSize)
    }
}

/// 界面全在 NshAppView 里; SwiftUI 只剩窗口框架, 以及 visionOS 的横竖屏 ornament。
struct HostRootView: View {
    #if os(visionOS)
    @EnvironmentObject private var windowSettings: WindowSettings
    #endif

    var body: some View {
        NshHostView(hooks: TestHooks.fromProcessInfo())
            .ignoresSafeArea(.keyboard)
            #if os(visionOS)
            .ornament(visibility: .visible, attachmentAnchor: .scene(.bottom)) {
                Button {
                    windowSettings.windowLandscape.toggle()
                } label: {
                    Label(
                        windowSettings.windowLandscape ? "横屏" : "竖屏",
                        systemImage: windowSettings.windowLandscape ? "rectangle.landscape" : "rectangle.portrait"
                    )
                }
                .accessibilityIdentifier("window-orientation")
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
                .glassBackgroundEffect()
            }
            #endif
    }
}

private struct AppWindowFrame: ViewModifier {
    @ObservedObject var settings: WindowSettings

    func body(content: Content) -> some View {
        #if os(visionOS)
        let size = settings.idealSize
        content
            .frame(
                minWidth: size.width,
                idealWidth: size.width,
                maxWidth: size.width,
                minHeight: size.height,
                idealHeight: size.height,
                maxHeight: size.height
            )
            .animation(.easeInOut(duration: 0.35), value: settings.windowLandscape)
        #else
        content
        #endif
    }
}
