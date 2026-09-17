import XCTest
@testable import ImageTranslator

final class WindowSettingsTests: XCTestCase {
    func testWindowLandscapeDefaultsToPortrait() {
        let suite = "WindowSettingsTests.default.\(UUID().uuidString)"
        guard let defaults = UserDefaults(suiteName: suite) else {
            return XCTFail("suite")
        }
        defaults.removePersistentDomain(forName: suite)
        defer { defaults.removePersistentDomain(forName: suite) }

        let store = WindowSettings(defaults: defaults, arguments: ["ImageTranslator"])
        XCTAssertEqual(store.windowLandscape, false)
    }

    func testWindowLandscapePersistsAcrossStoreInstances() {
        let suite = "WindowSettingsTests.roundtrip.\(UUID().uuidString)"
        guard let defaults = UserDefaults(suiteName: suite) else {
            return XCTFail("suite")
        }
        defaults.removePersistentDomain(forName: suite)
        defer { defaults.removePersistentDomain(forName: suite) }

        let store = WindowSettings(defaults: defaults, arguments: ["ImageTranslator"])
        store.windowLandscape = true

        let reloaded = WindowSettings(defaults: defaults, arguments: ["ImageTranslator"])
        XCTAssertEqual(reloaded.windowLandscape, true)
    }

    func testWindowLandscapeLaunchArgumentForcesOn() {
        let suite = "WindowSettingsTests.hook.\(UUID().uuidString)"
        guard let defaults = UserDefaults(suiteName: suite) else {
            return XCTFail("suite")
        }
        defaults.removePersistentDomain(forName: suite)
        defer { defaults.removePersistentDomain(forName: suite) }

        let store = WindowSettings(defaults: defaults, arguments: ["ImageTranslator", "-window-landscape"])
        XCTAssertEqual(store.windowLandscape, true)
    }
}
