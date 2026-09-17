import SwiftUI

@main
struct ImageTranslatorApp: App {
    @StateObject private var session = AppSession()
    @StateObject private var windowSettings = WindowSettings()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(session)
                .environmentObject(windowSettings)
                .modifier(AppWindowFrame(settings: windowSettings))
                .task { session.consumePendingHooks() }
        }
        .windowResizability(.contentSize)
        .defaultSize(windowSettings.idealSize)
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
