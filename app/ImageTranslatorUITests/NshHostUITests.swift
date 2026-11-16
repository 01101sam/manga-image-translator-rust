import ImageIO
import UIKit
import UniformTypeIdentifiers
import XCTest

/// 薄宿主 + NSH 界面的端到端测试。需要一个在跑的测试 Daemon: 环境变量 TEST_RUNNER_HOST_DAEMON_HOST / _PORT / _KEY 传进来。
final class NshHostUITests: XCTestCase {
    private var host: String { ProcessInfo.processInfo.environment["HOST_DAEMON_HOST"] ?? "127.0.0.1" }
    private var port: String { ProcessInfo.processInfo.environment["HOST_DAEMON_PORT"] ?? "18880" }
    private var key: String { ProcessInfo.processInfo.environment["HOST_DAEMON_KEY"] ?? "sam01101" }
    private var folder: String { ProcessInfo.processInfo.environment["HOST_PICK_FOLDER"] ?? "/tmp/nsh-ui-work/imgs" }

    /// 不需要 Daemon 的用例配到这个地址 (127.0.0.1:1, 没有人听): 配到测试 Daemon 的端口会因为假 Token 收到 401, 回到配对页。
    private let offlinePairing = ["-paired-test", "127.0.0.1", "1", "unused-token"]

    override func setUp() {
        continueAfterFailure = false
    }

    /// seed 为假时不播种 Daemon 地址, 让 App 自己走 Bonjour 发现。
    private func launch(_ extra: [String] = [], seed: Bool = true) -> XCUIApplication {
        let app = XCUIApplication()
        // HOST_BUNDLE: 用这份 bundle.js 代替 App 里内置的界面, 改 TSX 后不用重新构建 App (只在本地调试界面时用)。
        let bundle = ProcessInfo.processInfo.environment["HOST_BUNDLE"].map { ["-bundle", $0] } ?? []
        app.launchArguments = (seed ? ["-daemon-host", host, "-daemon-port", port] : []) + bundle + extra
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
        let app = launch(offlinePairing, seed: false)
        let add = element(app, "add-images")
        XCTAssertTrue(add.waitForExistence(timeout: 30))
        add.tap()
        for id in ["import-photos", "import-files", "import-pdf"] {
            XCTAssertTrue(element(app, id).waitForExistence(timeout: 5), "\(id) is missing from the import menu")
        }
        shot(app, "import-menu")
        // 点遮罩的上沿正中: 在卡片外面。按遮罩自己的坐标算, 不按窗口算 (visionOS 的窗口坐标点不到)。
        element(app, "import-backdrop").coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.1)).tap()
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
        let app = launch(offlinePairing + ["-seed-pasteboard", folder], seed: false)
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
        // App 启动后 XCUITest 发的第一个键盘事件到不了界面 (实测: 不碰菜单直接按 ⌘V 也不生效, 日志里 files.paste 少一次, 第二次才生效;
        // 原因没查清, 也不知道真键盘上会不会这样), 所以先按一次方向键 (只动列表的高亮, 没有别的作用) 再按 ⌘V。
        app.typeKey(XCUIKeyboardKey.downArrow, modifierFlags: [])
        // visionOS 的 typeKey 只收 XCUIKeyboardKey; iOS 上用 XCUIKeyboardKey("v") 发出的却不是 ⌘V (实测: 粘贴没有发生), 要用字符串那个重载。
        #if os(visionOS)
        app.typeKey(XCUIKeyboardKey(rawValue: "v"), modifierFlags: .command)
        #else
        app.typeKey("v", modifierFlags: .command)
        #endif
        expectation(for: NSPredicate(format: "label CONTAINS %@", "共 \(before + 2 * expected) 张"), evaluatedWith: summary)
        waitForExpectations(timeout: 30)
        shot(app, "pasted-with-command-v")
    }

    /// 在系统「照片」里分享第一张照片给本 App 的分享扩展, 等到它显示确认。照片 App 的界面随系统版本变: 第一张照片靠坐标点,
    /// 分享按钮的标识 PUOneUpBarButtonItemIdentifierShare 是实测的。要先用 xcrun simctl addmedia 给模拟器放几张照片, 并设 TEST_RUNNER_HOST_PHOTOS_SHARE=1。
    private func sharePhotoWithTheExtension() throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["HOST_PHOTOS_SHARE"] == "1", "needs a simulator with photos (xcrun simctl addmedia)")
        let photos = XCUIApplication(bundleIdentifier: "com.apple.mobileslideshow")
        photos.launch()
        let proceed = photos.buttons.matching(NSPredicate(format: "label IN %@", ["繼續", "继续", "Continue"])).firstMatch
        if proceed.waitForExistence(timeout: 3) { proceed.tap() }
        // 上一次留下的照片详情页会在重启后恢复: 已经有分享按钮就不再点第一张。
        let share = photos.buttons["PUOneUpBarButtonItemIdentifierShare"]
        if !share.waitForExistence(timeout: 3) { photos.coordinate(withNormalizedOffset: CGVector(dx: 0.12, dy: 0.15)).tap() }
        XCTAssertTrue(share.waitForExistence(timeout: 10), "Photos did not open a photo with a share button")
        share.tap()
        let entry = photos.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "图片翻译")).firstMatch
        XCTAssertTrue(entry.waitForExistence(timeout: 10), "the extension is not in the share sheet")
        shot(photos, "photos-share-sheet")
        entry.tap()
        let message = photos.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", "已加入")).firstMatch
        XCTAssertTrue(message.waitForExistence(timeout: 15), "the extension did not confirm")
        shot(photos, "extension-confirmation")
        // 扩展自己 1.2 秒后收起; 等它收起再回 App, 免得下一步在收起动画里点错
        sleep(3)
        photos.terminate()
    }

    /// 「打开方式」交来的文件: 系统用 URL 打开本 App 时 SwiftUI 的 onOpenURL 要接得住 (XCUIApplication.open 让系统把这个 URL 交给被测的 App),
    /// 并且走和拖放一样的导入。URL 在 App 容器外面, 所以只能拷不能搬: 原文件必须还在。不需要 Daemon。
    func testAFileOpenedWithTheAppIsImportedAndTheOriginalIsLeftAlone() throws {
        let scratch = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("it-open-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: scratch, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: scratch) }
        let pdf = scratch.appendingPathComponent("opened.pdf")
        try UIGraphicsPDFRenderer(bounds: CGRect(x: 0, y: 0, width: 300, height: 400)).writePDF(to: pdf) { $0.beginPage() }
        let app = launch(offlinePairing, seed: false)
        XCTAssertTrue(element(app, "add-images").waitForExistence(timeout: 30))
        app.open(pdf)
        XCTAssertTrue(element(app, "reader-status").waitForExistence(timeout: 30), "onOpenURL did not reach the reader")
        shot(app, "opened-with-the-app")
        XCTAssertTrue(FileManager.default.fileExists(atPath: pdf.path), "a file outside the app container must be copied, not moved")
    }

    func testSharingAPhotoFromPhotosShowsTheExtensionAndItConfirms() throws {
        try sharePhotoWithTheExtension()
    }

    /// 整条链: 照片 App 分享 -> 扩展放进真正的 App Group 收件箱 -> 打开 App 后这张照片进待上传。不需要 Daemon。
    func testAPhotoSharedFromPhotosShowsUpInTheAppAsAPendingUpload() throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["HOST_PHOTOS_SHARE"] == "1", "needs a simulator with photos (xcrun simctl addmedia)")
        // 先起一次 App, 把以前留在真收件箱里的条目搬掉, 之后的数才是这一次分享的。
        let cleaner = launch(offlinePairing, seed: false)
        XCTAssertTrue(element(cleaner, "add-images").waitForExistence(timeout: 30))
        cleaner.terminate()
        try sharePhotoWithTheExtension()
        let app = launch(offlinePairing, seed: false)
        let summary = element(app, "jobs-summary")
        XCTAssertTrue(summary.waitForExistence(timeout: 30))
        expectation(for: NSPredicate(format: "label CONTAINS %@", "共 1 张"), evaluatedWith: summary)
        waitForExpectations(timeout: 30)
        shot(app, "shared-photo-pending")
    }

    /// 一个临时的收件箱目录 (-inbox-dir), 里面按分享扩展的格式放条目; UI 测试以此代替真正的 App Group 容器。
    private func makeInbox() throws -> URL {
        let url = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("it-inbox-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }

    private func deposit(_ source: URL, as entry: String, into inbox: URL) throws {
        let directory = inbox.appendingPathComponent(entry, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try FileManager.default.copyItem(at: source, to: directory.appendingPathComponent(source.lastPathComponent))
    }

    private func pickFolderImages() throws -> [URL] {
        try FileManager.default.contentsOfDirectory(atPath: folder).sorted()
            .filter { ["png", "jpg", "jpeg"].contains(($0 as NSString).pathExtension.lowercased()) }
            .map { URL(fileURLWithPath: folder).appendingPathComponent($0) }
    }

    /// 分享扩展放进收件箱的图片: 冷启动时 TSX 还没挂好监听, 宿主要先攒着事件; 回到前台时再搬一次。不需要 Daemon。
    func testSharedImagesInTheInboxAreImportedAtLaunchAndAgainWhenTheAppComesBack() throws {
        let images = try pickFolderImages()
        XCTAssertGreaterThanOrEqual(images.count, 3, "the pick folder needs three images")
        let inbox = try makeInbox()
        try deposit(images[0], as: "0000000000001-0000-aaaaaaaa", into: inbox)
        try deposit(images[1], as: "0000000000001-0001-bbbbbbbb", into: inbox)
        let app = launch(offlinePairing + ["-inbox-dir", inbox.path], seed: false)
        let summary = element(app, "jobs-summary")
        XCTAssertTrue(summary.waitForExistence(timeout: 30))
        expectation(for: NSPredicate(format: "label CONTAINS %@", "共 2 张"), evaluatedWith: summary)
        waitForExpectations(timeout: 30)
        shot(app, "inbox-imported-at-launch")
        XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: inbox.path).filter { !$0.hasPrefix(".") }.isEmpty, "the inbox must be emptied")
        try deposit(images[2], as: "0000000000002-0000-cccccccc", into: inbox)
        XCUIDevice.shared.press(.home)
        app.activate()
        expectation(for: NSPredicate(format: "label CONTAINS %@", "共 3 张"), evaluatedWith: element(app, "jobs-summary"))
        waitForExpectations(timeout: 30)
        shot(app, "inbox-imported-on-return")
    }

    /// 一幅纯色图, 格式和方向由参数决定; 这台设备编不了这个格式时跳过。
    private func writeSolidImage(_ name: String, colour: UIColor, type: UTType, into folder: URL, width: Int = 400, height: Int = 600) throws -> URL {
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = true
        let image = UIGraphicsImageRenderer(size: CGSize(width: width, height: height), format: format).image { context in
            colour.setFill()
            context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        }
        let url = folder.appendingPathComponent(name)
        guard let cgImage = image.cgImage, let destination = CGImageDestinationCreateWithURL(url as CFURL, type.identifier as CFString, 1, nil) else {
            throw XCTSkip("this device cannot encode \(type.identifier)")
        }
        CGImageDestinationAddImage(destination, cgImage, [kCGImageDestinationLossyCompressionQuality: 0.9] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { throw XCTSkip("this device cannot encode \(type.identifier)") }
        return url
    }

    /// 截图里一个点附近 9x9 像素的平均颜色 (点按 app 的坐标, 截图按像素, 所以要按比例换算)。
    private func colour(in screenshot: XCUIScreenshot, at point: CGPoint, appWidth: CGFloat) throws -> (r: Int, g: Int, b: Int) {
        let image = try XCTUnwrap(screenshot.image.cgImage)
        let scale = CGFloat(image.width) / appWidth
        let rect = CGRect(x: point.x * scale - 4, y: point.y * scale - 4, width: 9, height: 9).integral
        let crop = try XCTUnwrap(image.cropping(to: rect))
        var pixel = [UInt8](repeating: 0, count: 4)
        let context = try XCTUnwrap(CGContext(data: &pixel, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.interpolationQuality = .medium
        context.draw(crop, in: CGRect(x: 0, y: 0, width: 1, height: 1))
        return (Int(pixel[0]), Int(pixel[1]), Int(pixel[2]))
    }

    /// 还没上传的图片没有别的图可显示, 条目靠宿主导入时生成的缩略图 (data URI) 显示自己; HEIC 在宿主里转成 JPEG, 名字的主干还在。
    /// 三幅纯色图 (PNG、HEIC、JPEG): 格子中间要是各自的颜色, 而不是占位图标的灰色。不需要 Daemon。
    func testPickedHEICAndSolidImagesShowTheirOwnThumbnailsBeforeTheyAreUploaded() throws {
        let scratch = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("it-thumbs-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: scratch, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: scratch) }
        _ = try writeSolidImage("red.png", colour: .red, type: .png, into: scratch)
        _ = try writeSolidImage("blue.heic", colour: .blue, type: .heic, into: scratch)
        _ = try writeSolidImage("green.jpg", colour: UIColor(red: 0, green: 0.8, blue: 0, alpha: 1), type: .jpeg, into: scratch)
        let app = launch(offlinePairing + ["-pick-folder", scratch.path], seed: false)
        XCTAssertTrue(element(app, "add-images").waitForExistence(timeout: 30))
        element(app, "add-images").tap()
        XCTAssertTrue(element(app, "import-files").waitForExistence(timeout: 10))
        element(app, "import-files").tap()
        let summary = element(app, "jobs-summary")
        expectation(for: NSPredicate(format: "label CONTAINS %@", "共 3 张"), evaluatedWith: summary)
        waitForExpectations(timeout: 30)
        func tile(_ name: String) -> XCUIElement {
            app.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH %@", name + ",")).firstMatch
        }
        for name in ["blue.jpg", "green.jpg", "red.png"] { XCTAssertTrue(tile(name).waitForExistence(timeout: 10), "\(name) is not in the list") }
        XCTAssertFalse(tile("blue.heic").exists, "the HEIC should have been converted and renamed")
        sleep(2)
        let screenshot = app.screenshot()
        shot(app, "thumbnails-before-upload")
        let width = app.frame.width
        for (name, expected) in [("red.png", (255, 0, 0)), ("blue.jpg", (0, 0, 255)), ("green.jpg", (0, 204, 0))] {
            let frame = tile(name).frame
            let found = try colour(in: screenshot, at: CGPoint(x: frame.midX, y: frame.minY + frame.height * 0.3), appWidth: width)
            XCTAssertLessThan(abs(found.r - expected.0), 70, "\(name) thumbnail red=\(found.r)")
            XCTAssertLessThan(abs(found.g - expected.1), 70, "\(name) thumbnail green=\(found.g)")
            XCTAssertLessThan(abs(found.b - expected.2), 70, "\(name) thumbnail blue=\(found.b)")
        }
    }

    /// 收件箱里有两个 PDF: 只打开第一个, 另一个被放掉并给一条提示。
    func testSharedPDFsOpenTheReaderOnlyOnceAndSayWhatWasIgnored() throws {
        let inbox = try makeInbox()
        let scratch = inbox.deletingLastPathComponent().appendingPathComponent("pdfs-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: scratch, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: scratch) }
        for (index, name) in ["one.pdf", "two.pdf"].enumerated() {
            let url = scratch.appendingPathComponent(name)
            try UIGraphicsPDFRenderer(bounds: CGRect(x: 0, y: 0, width: 300, height: 400)).writePDF(to: url) { $0.beginPage() }
            try deposit(url, as: "000000000000\(index)-0000-aaaaaaaa", into: inbox)
        }
        let app = launch(offlinePairing + ["-inbox-dir", inbox.path], seed: false)
        // 提示 3.5 秒就消失, 要先等它; 阅读器是常驻的, 晚一点也在。
        let toast = element(app, "toast")
        XCTAssertTrue(toast.waitForExistence(timeout: 30), "the ignored PDF should be announced")
        XCTAssertTrue(toast.label.contains("一次只打开一个 PDF"), "unexpected notice: \(toast.label)")
        XCTAssertTrue(element(app, "reader-status").waitForExistence(timeout: 30), "a shared PDF should open the reader")
        shot(app, "inbox-pdfs")
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

    // MARK: - 排序、拖动重排、条目菜单 (待上传的条目一样能排, 不需要 Daemon)

    /// 名字、大小、日期三种顺序互不相同的 8 张图 (自然序: cover, Page1, page2, page3, Page4, page10, page11, page20)。
    private var arrangeFolder: String { ProcessInfo.processInfo.environment["HOST_ARRANGE_FOLDER"] ?? "/tmp/sort-agent/fixture" }
    /// 几十张图, 列表放不下一屏, 用来拖到边缘自动滚动。
    private var manyFolder: String { ProcessInfo.processInfo.environment["HOST_MANY_FOLDER"] ?? "/tmp/sort-agent/many" }

    /// 一个条目一个按钮, 标签是「名字, 状态」(等待上传、已完成……)。夹具里的名字都以 .png 结尾。
    private let rowLabel = NSPredicate(format: "label MATCHES %@", #".*\.png, [^,]+"#)

    /// 任务页上看得见的条目标题, 按屏幕位置排 (先上后下, 再左右)。NSH 的列表在可见范围外还留着几十个已渲染的 cell
    /// (实测 60 张图时辅助功能树里有 49 个, frame 在列表上下很远的地方), 所以只取中心落在列表区域里的。
    private func titles(_ app: XCUIApplication) -> [String] {
        let list = element(app, "job-list")
        let area = (list.exists ? list : element(app, "job-grid")).frame
        let rows = app.descendants(matching: .any).matching(rowLabel).allElementsBoundByIndex.filter { $0.frame.width > 0 && area.contains(CGPoint(x: $0.frame.midX, y: $0.frame.midY)) }
        return rows.sorted { ($0.frame.minY, $0.frame.minX) < ($1.frame.minY, $1.frame.minX) }.map { String($0.label.split(separator: ",")[0]) }
    }

    private func row(_ app: XCUIApplication, _ title: String) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH %@", title + ", ")).firstMatch
    }

    /// 配成离线 (假 Token), 从目录里选图进待上传。layout 为 "list" 时 iPad 也排成一行一项。
    private func launchWithImages(count: Int, folder: String, layout: String? = nil) -> XCUIApplication {
        let app = launch(offlinePairing + ["-pick-folder", folder] + (layout.map { ["-layout", $0] } ?? []), seed: false)
        let add = element(app, "add-images")
        XCTAssertTrue(add.waitForExistence(timeout: 30))
        add.tap()
        let files = element(app, "import-files")
        XCTAssertTrue(files.waitForExistence(timeout: 5))
        files.tap()
        let summary = element(app, "jobs-summary")
        expectation(for: NSPredicate(format: "label CONTAINS %@", "共 \(count) 张"), evaluatedWith: summary)
        waitForExpectations(timeout: 30)
        return app
    }

    private func sort(_ app: XCUIApplication, _ choice: String) {
        element(app, "open-sort").tap()
        let entry = element(app, "sort-\(choice)")
        XCTAssertTrue(entry.waitForExistence(timeout: 5), "sort-\(choice) is missing from the sort menu")
        entry.tap()
        expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: element(app, "sort-menu"))
        waitForExpectations(timeout: 5)
    }

    private let natural = ["cover.png", "Page1.png", "page2.png", "page3.png", "Page4.png", "page10.png", "page11.png", "page20.png"]

    func testSortMenuOrdersTheListByNameDateAndSizeBothWays() {
        let app = launchWithImages(count: 8, folder: arrangeFolder)
        XCTAssertEqual(titles(app), natural, "a folder comes in natural order")
        attach(app.screenshot(), "arrange-initial")
        // 大小 (字节): page11 < page2 < cover < Page4 < page10 < page3 < page20 < Page1; 日期 (旧到新): Page1, page3, page20, page10, page11, Page4, cover, page2
        let bySize = ["page11.png", "page2.png", "cover.png", "Page4.png", "page10.png", "page3.png", "page20.png", "Page1.png"]
        let byDate = ["Page1.png", "page3.png", "page20.png", "page10.png", "page11.png", "Page4.png", "cover.png", "page2.png"]
        sort(app, "size-asc")
        XCTAssertEqual(titles(app), bySize)
        attach(app.screenshot(), "arrange-size-asc")
        sort(app, "size-desc")
        XCTAssertEqual(titles(app), bySize.reversed())
        sort(app, "modified-asc")
        XCTAssertEqual(titles(app), byDate)
        attach(app.screenshot(), "arrange-date-asc")
        sort(app, "modified-desc")
        XCTAssertEqual(titles(app), byDate.reversed())
        sort(app, "name-desc")
        XCTAssertEqual(titles(app), natural.reversed())
        sort(app, "name-asc")
        XCTAssertEqual(titles(app), natural)
        attach(app.screenshot(), "arrange-name-asc")
    }

    func testItemMenuMovesAnItemToTheFrontOrTheBack() {
        let app = launchWithImages(count: 8, folder: arrangeFolder)
        element(app, "更多: cover.png").tap()
        XCTAssertTrue(element(app, "item-menu").waitForExistence(timeout: 5))
        XCTAssertFalse(element(app, "move-to-front").isEnabled, "cover.png is already first")
        attach(app.screenshot(), "arrange-item-menu")
        element(app, "move-to-back").tap()
        XCTAssertEqual(titles(app), Array(natural.dropFirst()) + ["cover.png"])
        element(app, "更多: page11.png").tap()
        XCTAssertTrue(element(app, "move-to-front").waitForExistence(timeout: 5))
        element(app, "move-to-front").tap()
        XCTAssertEqual(titles(app), ["page11.png"] + Array(natural.dropFirst().filter { $0 != "page11.png" }) + ["cover.png"])
    }

    /// 拖动中途 (手指按在目标上不动) 看到的影子和指示线, 以及当时的截图。press(thenHoldForDuration:) 会一直占着主线程, 所以从后台线程查。
    private struct MidDrag {
        var ghost: CGRect?
        var marker: CGRect?
        var rows: [CGRect] = []
        var screenshot: XCUIScreenshot?
    }

    /// 长按再拖: 按住 source 的中心, 拖到 target 的中心, 在那里停 hold 秒再放手; 停住的第 hold / 2 秒从后台线程看一眼界面。
    private func longPressDrag(_ app: XCUIApplication, _ source: XCUIElement, to target: XCUIElement, hold: TimeInterval = 6) -> MidDrag {
        let from = source.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let to = target.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let seen = Locked(MidDrag())
        let looked = expectation(description: "looked at the screen during the drag")
        // 按 1 秒、拖约 1.5 秒, 然后停; 停住之后再等一会儿看。
        DispatchQueue.global().asyncAfter(deadline: .now() + 1.0 + 2.5 + hold / 2) {
            var value = MidDrag()
            // 先截图: 查辅助功能树要几秒, 截图排在后面就拍到放手之后了。
            value.screenshot = XCUIScreen.main.screenshot()
            value.ghost = app.descendants(matching: .any).matching(identifier: "drag-ghost").firstMatch.frameIfExists
            value.marker = app.descendants(matching: .any).matching(identifier: "drop-marker").firstMatch.frameIfExists
            seen.set(value)
            looked.fulfill()
        }
        from.press(forDuration: 1.0, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: hold)
        wait(for: [looked], timeout: 30)
        return seen.get()
    }

    private func attach(_ screenshot: XCUIScreenshot?, _ name: String) {
        guard let screenshot else { return }
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        let path = (ProcessInfo.processInfo.environment["HOST_SHOTS"] ?? "/tmp/sort-agent/shots") + "/\(name).png"
        try? FileManager.default.createDirectory(atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
        try? screenshot.pngRepresentation.write(to: URL(fileURLWithPath: path))
    }

    /// 网格 (iPad) 里长按拖一张到另一张的位置; 拖动没有碰上点按, 所以没有打开查看器。
    func testLongPressDragMovesATileInTheGrid() throws {
        let app = launchWithImages(count: 8, folder: arrangeFolder)
        XCTAssertTrue(element(app, "job-grid").exists, "an iPad shows the grid")
        let page1 = row(app, "Page1.png")
        let page10 = row(app, "page10.png")
        let page10Frame = page10.frame
        let mid = longPressDrag(app, page1, to: page10)
        attach(mid.screenshot, "arrange-grid-mid-drag")
        print("MIDDRAG grid ghost=\(String(describing: mid.ghost)) marker=\(String(describing: mid.marker)) target=\(page10Frame)")
        // 影子跟着手指: 手指按在源格中心、停在 page10 的中心, 所以影子正好盖在 page10 上 (左上角重合; 按钮的 frame 不含底部的操作栏, 所以不比中心);
        // 指示线是 page10 后缘外 4 点 (格间距的一半) 的一条竖线, 和一整格一样高。
        let ghost = try XCTUnwrap(mid.ghost, "no drag ghost while the finger rests on the target")
        XCTAssertEqual(ghost.minX, page10Frame.minX, accuracy: 6)
        XCTAssertEqual(ghost.minY, page10Frame.minY, accuracy: 6)
        let marker = try XCTUnwrap(mid.marker, "no drop marker while the finger rests on the target")
        XCTAssertEqual(marker.midX, page10Frame.maxX + 4, accuracy: 6, "the marker sits in the gap after the tile it lands on")
        XCTAssertEqual(marker.height, ghost.height, accuracy: 4, "the marker is as tall as a tile")
        XCTAssertEqual(marker.minY, page10Frame.minY, accuracy: 6)
        attach(app.screenshot(), "arrange-grid-after-drop")
        XCTAssertEqual(titles(app), ["cover.png", "page2.png", "page3.png", "Page4.png", "page10.png", "Page1.png", "page11.png", "page20.png"])
        XCTAssertFalse(element(app, "drag-ghost").exists, "the ghost goes away at the drop")
        XCTAssertFalse(element(app, "drop-marker").exists)
        XCTAssertFalse(element(app, "viewer-close").exists, "letting go of a long press must not open anything")
    }

    /// 同样的拖动, 排成一行一项 (手机的样子)。
    func testLongPressDragMovesARowInTheList() throws {
        let app = launchWithImages(count: 8, folder: arrangeFolder, layout: "list")
        XCTAssertTrue(element(app, "job-list").exists)
        let last = row(app, "page20.png")
        let third = row(app, "page2.png")
        let list = element(app, "job-list").frame
        let mid = longPressDrag(app, last, to: third)
        attach(mid.screenshot, "arrange-list-mid-drag")
        print("MIDDRAG list ghost=\(String(describing: mid.ghost)) marker=\(String(describing: mid.marker)) list=\(list)")
        // 往上拖: 指示线是目标那一行上缘的一条横线, 整行宽。第 2 行的上缘在列表上沿往下 2 行 (每行 72 点) 的地方。
        let ghost = try XCTUnwrap(mid.ghost, "no drag ghost while the finger rests on the target")
        XCTAssertEqual(ghost.height, 72, accuracy: 2)
        XCTAssertEqual(ghost.width, list.width, accuracy: 2)
        let marker = try XCTUnwrap(mid.marker, "no drop marker while the finger rests on the target")
        XCTAssertEqual(marker.midY, list.minY + 2 * 72, accuracy: 4, "the marker sits on the top edge of the row it lands on")
        XCTAssertEqual(marker.width, list.width, accuracy: 2)
        attach(app.screenshot(), "arrange-list-after-drop")
        XCTAssertEqual(titles(app), ["cover.png", "Page1.png", "page20.png", "page2.png", "page3.png", "Page4.png", "page10.png", "page11.png"])
        XCTAssertFalse(element(app, "drag-ghost").exists)
    }

    /// 重启后顺序保留: 排序和移动之后退出再打开, 列表按用户看到的顺序恢复 (Job 随条目存了名字、大小和日期)。
    /// 需要一个在跑的、没有同名旧 Job 的 Daemon 替身或测试 Daemon (HOST_DAEMON_PORT): 只有已提交的 Job 会被保存, 待上传的图片重启后本来就没有了;
    /// 上一轮留在 App 沙盒里的存档会按 id 去 Daemon 找, 找不到就丢弃, 所以重跑前换一个新的 Daemon 进程。
    func testTheArrangedOrderSurvivesARelaunch() {
        let first = launch(["-forget-pairing", "-pick-folder", arrangeFolder])
        pair(first)
        element(first, "add-images").tap()
        XCTAssertTrue(element(first, "import-files").waitForExistence(timeout: 5))
        element(first, "import-files").tap()
        let summary = element(first, "jobs-summary")
        expectation(for: NSPredicate(format: "label CONTAINS %@ AND label CONTAINS %@", "全部完成", "共 8 张"), evaluatedWith: summary)
        waitForExpectations(timeout: 60)
        sort(first, "size-desc")
        element(first, "更多: page11.png").tap()
        XCTAssertTrue(element(first, "move-to-front").waitForExistence(timeout: 5))
        element(first, "move-to-front").tap()
        let arranged = titles(first)
        XCTAssertEqual(arranged.first, "page11.png")
        XCTAssertEqual(arranged.count, 8)
        attach(first.screenshot(), "arrange-before-relaunch")
        // 存盘有 0.4 秒的防抖。
        sleep(2)
        first.terminate()

        let second = launch()
        let again = element(second, "jobs-summary")
        XCTAssertTrue(again.waitForExistence(timeout: 30))
        expectation(for: NSPredicate(format: "label CONTAINS %@", "共 8 张"), evaluatedWith: again)
        waitForExpectations(timeout: 30)
        XCTAssertEqual(titles(second), arranged, "the list comes back in the order the user left it")
        attach(second.screenshot(), "arrange-after-relaunch")
        // 大小和日期也回来了: 再按日期排序能用。
        sort(second, "modified-asc")
        XCTAssertEqual(titles(second), ["Page1.png", "page3.png", "page20.png", "page10.png", "page11.png", "Page4.png", "cover.png", "page2.png"])
        // 已完成的 Job 也能拖, 放手的那一下不会把它打开。
        let finished = titles(second)
        _ = longPressDrag(second, row(second, finished[1]), to: row(second, finished[0]), hold: 1)
        XCTAssertFalse(element(second, "viewer-close").exists, "dragging a finished job must not open it")
        XCTAssertEqual(Array(titles(second).prefix(2)), [finished[1], finished[0]])
    }

    /// 长按之后不动就抬手: 没有影子留下, 没有改顺序, 也没有打开东西, 列表还能照常滚动 (拖动开始时 List 的 scrollable 关了, 结束时要恢复)。
    func testALongPressWithoutMovingLeavesNothingBehindAndTheListStillScrolls() {
        let app = launchWithImages(count: 60, folder: manyFolder, layout: "list")
        let before = Array(titles(app).prefix(6))
        XCTAssertEqual(before.first, "img01.png")
        row(app, "img03.png").coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).press(forDuration: 1.5)
        XCTAssertFalse(element(app, "drag-ghost").exists)
        XCTAssertFalse(element(app, "drop-marker").exists)
        XCTAssertEqual(Array(titles(app).prefix(6)), before)
        let list = element(app, "job-list")
        list.swipeUp()
        XCTAssertNotEqual(titles(app).first, "img01.png", "the list should scroll again after a long press that did not move")
    }

    /// 滚动之后再拖: 落点按滚动后的位置算, 不是按列表顶部算。
    func testDraggingAfterScrollingUsesTheScrolledPosition() {
        let app = launchWithImages(count: 60, folder: manyFolder, layout: "list")
        let list = element(app, "job-list")
        list.swipeUp()
        list.swipeUp()
        let visible = Array(titles(app).prefix(8))
        XCTAssertGreaterThanOrEqual(visible.count, 8)
        XCTAssertNotEqual(visible.first, "img01.png")
        _ = longPressDrag(app, row(app, visible[2]), to: row(app, visible[5]), hold: 1)
        var expected = visible
        expected.insert(expected.remove(at: 2), at: 5)
        XCTAssertEqual(Array(titles(app).prefix(8)), expected)
    }

    /// 普通的上下滑动照常滚动列表, 不会被当成拖动 (没有长按)。
    func testAnOrdinarySwipeStillScrollsAndDoesNotReorder() {
        let app = launchWithImages(count: 60, folder: manyFolder, layout: "list")
        let before = titles(app)
        XCTAssertEqual(before.first, "img01.png")
        let list = element(app, "job-list")
        list.swipeUp()
        list.swipeUp()
        let scrolled = titles(app)
        XCTAssertNotEqual(scrolled.first, "img01.png", "swiping should scroll the list")
        XCTAssertEqual(scrolled, scrolled.sorted { $0.localizedStandardCompare($1) == .orderedAscending }, "a swipe must not have moved anything")
    }

    /// 拖到列表下沿停住, 列表自动往下滚, 放手时那一项落在远处。
    func testDraggingToTheBottomEdgeAutoScrollsTheListAndDropsFarAway() {
        let app = launchWithImages(count: 60, folder: manyFolder, layout: "list")
        let first = row(app, "img01.png")
        XCTAssertTrue(first.exists)
        let list = element(app, "job-list")
        let from = first.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let edge = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.99))
        from.press(forDuration: 1.0, thenDragTo: edge, withVelocity: .slow, thenHoldForDuration: 3)
        attach(app.screenshot(), "arrange-after-autoscroll-drop")
        let nowVisible = titles(app)
        XCTAssertFalse(nowVisible.isEmpty)
        // 放手时指针在列表下沿, 那一项落在下沿那一格: 半截在列表外, 所以不在「看得见」的集合里, 但它在已渲染的范围内; 回到顶部才渲染的话它就是没动。
        XCTAssertTrue(row(app, "img01.png").exists, "the dragged row dropped where the list had scrolled to, not back at the top")
        XCTAssertNotEqual(nowVisible.first, "img02.png", "the list scrolled away from the top while the finger rested at the edge")
        for _ in 0..<12 { list.swipeDown() }
        XCTAssertEqual(titles(app).first, "img02.png", "img01 left the top of the list")
    }
}

private extension XCUIElement {
    var frameIfExists: CGRect? { exists ? frame : nil }
}

private final class Locked<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var value: Value
    init(_ value: Value) { self.value = value }
    func get() -> Value { lock.lock(); defer { lock.unlock() }; return value }
    func set(_ next: Value) { lock.lock(); value = next; lock.unlock() }
}
