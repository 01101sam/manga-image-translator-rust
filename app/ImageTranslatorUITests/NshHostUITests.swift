import XCTest

/// 薄宿主 + NSH 界面的端到端测试。需要一个在跑的测试 Daemon: 环境变量 TEST_RUNNER_HOST_DAEMON_HOST / _PORT / _KEY 传进来。
final class NshHostUITests: XCTestCase {
    private var host: String { ProcessInfo.processInfo.environment["HOST_DAEMON_HOST"] ?? "127.0.0.1" }
    private var port: String { ProcessInfo.processInfo.environment["HOST_DAEMON_PORT"] ?? "18880" }
    private var key: String { ProcessInfo.processInfo.environment["HOST_DAEMON_KEY"] ?? "sam01101" }
    private var folder: String { ProcessInfo.processInfo.environment["HOST_PICK_FOLDER"] ?? "/tmp/nsh-ui-work/imgs" }

    override func setUp() {
        continueAfterFailure = false
    }

    /// seed 为假时不播种 Daemon 地址, 让 App 自己走 Bonjour 发现。
    private func launch(_ extra: [String] = [], seed: Bool = true) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = (seed ? ["-daemon-host", host, "-daemon-port", port] : []) + extra
        app.launch()
        return app
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    /// 摘要里的任务数: "还没有任务" 是 0, "共 N 张 · ..." 是 N。
    private static func jobCount(_ summary: String) -> Int {
        guard let range = summary.range(of: #"共 (\d+) 张"#, options: .regularExpression) else { return 0 }
        return Int(summary[range].filter(\.isNumber)) ?? 0
    }

    private func shot(_ app: XCUIApplication, _ name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func pair(_ app: XCUIApplication) {
        let field = element(app, "pairing-key")
        XCTAssertTrue(field.waitForExistence(timeout: 30), "pairing screen did not show the key field")
        shot(app, "pairing-screen")
        add(XCTAttachment(string: app.debugDescription))
        // TSX 把输入框预填成默认的 Pairing Key; 值不对才清掉重输。
        let input = app.textFields.firstMatch
        let current = (input.value as? String) ?? ""
        if current != key {
            input.tap()
            input.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: current.count))
            input.typeText(key)
        }
        element(app, "pairing-confirm").tap()
        XCTAssertTrue(element(app, "add-images").waitForExistence(timeout: 30), "pairing did not reach the jobs page")
    }

    func testPairingIsRememberedAcrossRelaunch() {
        let first = launch(["-forget-pairing"])
        pair(first)
        shot(first, "paired")
        first.terminate()

        let second = launch()
        XCTAssertTrue(element(second, "add-images").waitForExistence(timeout: 30), "relaunch should go straight to the jobs page")
        XCTAssertFalse(element(second, "pairing-key").exists, "relaunch asked for the Pairing Key again")
        shot(second, "relaunch-without-pairing")
    }

    /// 需要先在 Mac 上用 dns-sd -R 公布一个 _imagetranslator._tcp 实例; TEST_RUNNER_HOST_BONJOUR=1 才跑。
    func testBonjourDiscoversTheDaemonThenPairs() throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["HOST_BONJOUR"] == "1", "needs a Bonjour-advertised test Daemon")
        let app = launch(["-forget-pairing"], seed: false)
        pair(app)
        shot(app, "bonjour-paired")
        app.terminate()
        let again = launch([], seed: false)
        XCTAssertTrue(element(again, "add-images").waitForExistence(timeout: 30))
        XCTAssertFalse(element(again, "pairing-key").exists)
    }

    /// 从 App 内的可拖方块发起拖放, 落到 NSH 界面上, 走真正的 UIDropInteraction。
    func testDraggingFilesOntoTheWindowAddsThem() {
        let app = launch(["-forget-pairing", "-debug-drag-source", folder])
        pair(app)
        let source = element(app, "debug-drag-source")
        XCTAssertTrue(source.waitForExistence(timeout: 10))
        let summary = element(app, "jobs-summary")
        XCTAssertTrue(summary.waitForExistence(timeout: 10))
        // Daemon 上可能还留着之前的 Job, 所以数增量。
        let before = Self.jobCount(summary.label)
        let target = app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        source.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).press(forDuration: 1.5, thenDragTo: target)
        shot(app, "after-drop")
        let added = NSPredicate(format: "label CONTAINS %@", "共 \(before + 3) 张")
        expectation(for: added, evaluatedWith: summary)
        waitForExpectations(timeout: 60)
    }

    /// PDF 阅读器: 宿主的 pdf.pick、it.pdf 原生组件、pdf.renderPage 与上传连起来跑一遍。需要 TEST_RUNNER_HOST_TOKEN (已配对的 Token)。
    func testReaderOpensAPdfTurnsPagesAndTranslatesOne() throws {
        let token = try XCTUnwrap(ProcessInfo.processInfo.environment["HOST_TOKEN"], "needs HOST_TOKEN")
        let pdf = ProcessInfo.processInfo.environment["HOST_PICK_PDF"] ?? "/tmp/nsh-spike/serve/doc100.pdf"
        let app = launch(["-paired-test", host, port, token, "-section", "reader", "-pick-pdf", pdf], seed: false)
        let open = element(app, "reader-open")
        XCTAssertTrue(open.waitForExistence(timeout: 30))
        open.tap()
        XCTAssertTrue(element(app, "pdf-native-view").waitForExistence(timeout: 30), "the PDFView native component did not appear")
        let status = element(app, "reader-status")
        XCTAssertTrue(status.waitForExistence(timeout: 10))
        let first = status.label
        shot(app, "reader-page-1")
        element(app, "reader-next").tap()
        let moved = NSPredicate(format: "label != %@", first)
        expectation(for: moved, evaluatedWith: status)
        waitForExpectations(timeout: 10)
        shot(app, "reader-page-2")
        element(app, "reader-space").tap()
        let done = NSPredicate(format: "label CONTAINS %@", "已翻译")
        expectation(for: done, evaluatedWith: status)
        waitForExpectations(timeout: 180)
        element(app, "reader-space").tap()
        let showing = NSPredicate(format: "label CONTAINS %@", "已翻译 · 译文")
        expectation(for: showing, evaluatedWith: status)
        waitForExpectations(timeout: 10)
        sleep(2)
        shot(app, "reader-translated")
        // 导出: 宿主逐页下载译文、拼出 PDF、弹分享面板。
        element(app, "reader-export").tap()
        XCTAssertTrue(element(app, "ActivityListView").waitForExistence(timeout: 120), "the share sheet for the exported PDF did not appear")
        shot(app, "reader-export-share")
    }

    func testPickedImagesUploadFinishAndOpenInViewer() {
        let app = launch(["-forget-pairing", "-pick-folder", folder])
        pair(app)
        element(app, "add-images").tap()
        let files = element(app, "import-files")
        XCTAssertTrue(files.waitForExistence(timeout: 10), "the import menu did not open")
        shot(app, "import-menu")
        files.tap()
        let summary = element(app, "jobs-summary")
        XCTAssertTrue(summary.waitForExistence(timeout: 30))
        let finished = NSPredicate(format: "label CONTAINS %@ AND label CONTAINS %@", "共", "全部完成")
        expectation(for: finished, evaluatedWith: summary)
        waitForExpectations(timeout: 600)
        shot(app, "all-done")
        let row = element(app, "job-row")
        XCTAssertTrue(row.exists)
        row.tap()
        sleep(3)
        shot(app, "viewer")
        element(app, "viewer-close").tap()
        XCTAssertTrue(element(app, "add-images").waitForExistence(timeout: 10), "closing the viewer should show the Jobs list again")
        XCTAssertFalse(element(app, "viewer-close").exists)
    }

    /// 导入菜单是自己画的 (嵌入运行时没有 Menu、Popover、Sheet), 要在真实的 iOS 运行时里确认: 能打开、列出宿主声明的来源、点卡片外收起。不需要 Daemon。
    func testImportMenuOpensListsTheHostsSourcesAndClosesOnATapOutside() {
        let app = launch(["-paired-test", host, port, "unused-token"], seed: false)
        let add = element(app, "add-images")
        XCTAssertTrue(add.waitForExistence(timeout: 30))
        add.tap()
        for id in ["import-photos", "import-files", "import-pdf"] {
            XCTAssertTrue(element(app, id).waitForExistence(timeout: 5), "\(id) is missing from the import menu")
        }
        shot(app, "import-menu")
        app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.04, dy: 0.04)).tap()
        let gone = NSPredicate(format: "exists == false")
        expectation(for: gone, evaluatedWith: element(app, "import-menu"))
        waitForExpectations(timeout: 5)
        XCTAssertTrue(add.exists, "the Jobs page should still be there after closing the menu")
    }

    /// 粘贴: -seed-pasteboard 让 App 自己把图片放进系统剪贴板 (自己写的, 读的时候不弹「允许粘贴」), 再从导入菜单和 ⌘V 各粘一次。不需要 Daemon。
    func testPasteFromTheMenuAndWithCommandVAddsTheClipboard() throws {
        let names = try FileManager.default.contentsOfDirectory(atPath: folder)
        let expected = names.filter { ["png", "jpg", "jpeg"].contains(($0 as NSString).pathExtension.lowercased()) }.count
        XCTAssertGreaterThan(expected, 0, "the pick folder has no images")
        let app = launch(["-paired-test", host, port, "unused-token", "-seed-pasteboard", folder], seed: false)
        let add = element(app, "add-images")
        XCTAssertTrue(add.waitForExistence(timeout: 30))
        let summary = element(app, "jobs-summary")
        XCTAssertTrue(summary.waitForExistence(timeout: 10))
        let before = Self.jobCount(summary.label)
        add.tap()
        let paste = element(app, "import-paste")
        XCTAssertTrue(paste.waitForExistence(timeout: 5), "the import menu has no paste item")
        paste.tap()
        expectation(for: NSPredicate(format: "label CONTAINS %@", "共 \(before + expected) 张"), evaluatedWith: summary)
        waitForExpectations(timeout: 30)
        shot(app, "pasted-from-menu")
        app.typeKey("v", modifierFlags: .command)
        expectation(for: NSPredicate(format: "label CONTAINS %@", "共 \(before + 2 * expected) 张"), evaluatedWith: summary)
        waitForExpectations(timeout: 30)
        shot(app, "pasted-with-command-v")
    }

    /// 配置页: 从 Daemon 读出配置并显示 (Pairing Key 在其中), Engine 状态也在。保存条只在有修改时才出现。
    func testConfigPageShowsTheDaemonSettings() throws {
        let token = try XCTUnwrap(ProcessInfo.processInfo.environment["HOST_TOKEN"], "needs HOST_TOKEN")
        let app = launch(["-paired-test", host, port, token, "-section", "config"], seed: false)
        XCTAssertTrue(element(app, "cfg-pairing_key").waitForExistence(timeout: 30), "the Config page did not load the settings")
        XCTAssertTrue(element(app, "engine-state").exists)
        XCTAssertFalse(element(app, "config-error").exists)
        shot(app, "config")
    }

    /// 在三个分区之间来回切 30 圈: 每次切换后都能看到目标分区的内容, 没有卡死。
    func testSwitchingSectionsRepeatedlyStaysResponsive() throws {
        let token = try XCTUnwrap(ProcessInfo.processInfo.environment["HOST_TOKEN"], "needs HOST_TOKEN")
        let app = launch(["-paired-test", host, port, token], seed: false)
        XCTAssertTrue(element(app, "add-images").waitForExistence(timeout: 30))
        let sections = [("tab-jobs", "add-images"), ("tab-reader", "reader-open"), ("tab-config", "engine-state")]
        for cycle in 1...30 {
            for (tab, marker) in sections {
                element(app, tab).tap()
                XCTAssertTrue(element(app, marker).waitForExistence(timeout: 5), "\(tab) did not show \(marker) at cycle \(cycle)")
            }
        }
        shot(app, "sections-after-30-cycles")
    }
}
