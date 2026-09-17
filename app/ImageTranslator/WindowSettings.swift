import Combine
import Foundation

final class WindowSettings: ObservableObject {
    static let landscapeKey = "windowLandscape"
    static let portraitSize = CGSize(width: 720, height: 960)
    static let landscapeSize = CGSize(width: 1080, height: 720)

    private let defaults: UserDefaults

    @Published var windowLandscape: Bool {
        didSet { defaults.set(windowLandscape, forKey: Self.landscapeKey) }
    }

    var idealSize: CGSize {
        windowLandscape ? Self.landscapeSize : Self.portraitSize
    }

    init(defaults: UserDefaults = .standard, arguments: [String] = ProcessInfo.processInfo.arguments) {
        self.defaults = defaults
        if arguments.contains("-window-landscape") {
            windowLandscape = true
            defaults.set(true, forKey: Self.landscapeKey)
        } else {
            windowLandscape = defaults.bool(forKey: Self.landscapeKey)
        }
    }
}
