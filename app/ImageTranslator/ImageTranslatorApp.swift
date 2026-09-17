import SwiftUI

@main
struct ImageTranslatorApp: App {
    @StateObject private var session = AppSession()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(session)
                .task { session.consumePendingHooks() }
        }
    }
}
