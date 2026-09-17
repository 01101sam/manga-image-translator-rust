import XCTest

final class ImageTranslatorUITests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testDiscoveryAndPairing() throws {
        let env = try UITestEnv.load()
        let app = XCUIApplication()
        app.launchArguments = ["-daemon-host", env.host, "-daemon-port", "\(env.port)"]
        app.launch()

        let row = app.buttons["daemon-row"]
        XCTAssertTrue(row.waitForExistence(timeout: 10), "discovered daemon row")
        attachShot(app, name: "discovery-row")
        try? "".write(toFile: env.pairingCodeFile, atomically: true, encoding: .utf8)
        row.tap()

        let field = app.textFields["pairing-code"]
        XCTAssertTrue(field.waitForExistence(timeout: 10), "code field after request")
        let code = try waitForPairingCode(path: env.pairingCodeFile, timeout: 30)
        field.tap()
        field.typeText(code)
        app.buttons["pairing-confirm"].tap()

        XCTAssertTrue(app.tabBars.buttons["任务"].waitForExistence(timeout: 15), "tab bar after pairing")
        XCTAssertTrue(app.tabBars.buttons["阅读"].exists)
        XCTAssertTrue(app.tabBars.buttons["配置"].exists)
        attachShot(app, name: "paired-tabs")
    }

    func testJobSubmitAndDone() throws {
        let env = try UITestEnv.load()
        let token = try env.tokenOrPair()
        let app = XCUIApplication()
        app.launchArguments = [
            "-paired-test", env.host, "\(env.port)", token,
            "-auto-submit-image", "sample.jpg",
        ]
        app.launch()
        defer { attachShot(app, name: "job-submit-end") }

        let args = app.descendants(matching: .any)["launch-args"]
        _ = args.waitForExistence(timeout: 10)
        let row = app.descendants(matching: .any)["job-row"]
        XCTAssertTrue(row.waitForExistence(timeout: 25), "job row after auto submit; args=\(args.label); banner=\(bannerLabel(app))")
        XCTAssertTrue(wait(for: { self.rowValues(app).contains { $0.contains("已完成") } }, timeout: 90), "done; \(rowValues(app))")
        attachShot(app, name: "job-done")
    }

    func testPDFReaderKeysAndSpace() throws {
        let env = try UITestEnv.load()
        let token = try env.tokenOrPair()
        let app = XCUIApplication()
        app.launchArguments = [
            "-paired-test", env.host, "\(env.port)", token,
            "-open-test-pdf", "three-pages.pdf",
        ]
        app.launch()

        let status = app.staticTexts["reader-status"]
        XCTAssertTrue(status.waitForExistence(timeout: 15), "reader status")
        XCTAssertTrue(status.label.contains("第 1/3 页"), status.label)
        attachShot(app, name: "reader-page-1")

        flipForward(app, status: status)
        XCTAssertTrue(status.label.contains("第 2/3 页"), status.label)
        attachShot(app, name: "reader-page-2")

        pressSpace(app)
        XCTAssertTrue(
            wait(for: status, matching: { $0.contains("排队中") || $0.contains("翻译中") || $0.contains("译文") || $0.contains("原文") && !$0.contains("未翻译") }, timeout: 15),
            status.label
        )

        XCTAssertTrue(wait(for: status, matching: { $0.contains("译文") || $0.contains("原文") }, timeout: 90), status.label)
        let before = status.label
        pressSpace(app)
        XCTAssertTrue(wait(for: status, matching: { $0 != before && ($0.contains("译文") || $0.contains("原文")) }, timeout: 10), status.label)
        attachShot(app, name: "reader-toggle")
    }

    func testFolderBatchImport() throws {
        let env = try UITestEnv.load()
        let token = try env.tokenOrPair()
        let folder = env.batchFolder ?? "batch"
        let app = XCUIApplication()
        app.launchArguments = [
            "-paired-test", env.host, "\(env.port)", token,
            "-auto-import-folder", folder,
        ]
        app.launch()
        defer { attachShot(app, name: "folder-batch") }

        let rows = app.descendants(matching: .any).matching(identifier: "job-row")
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline, rows.count < 5 {
            RunLoop.current.run(until: Date().addingTimeInterval(0.4))
        }
        XCTAssertEqual(rows.count, 5, "folder import should submit 5 jobs; banner=\(bannerLabel(app))")
    }

    func testCancelQueuedJob() throws {
        let env = try UITestEnv.load()
        let token = try env.tokenOrPair()
        let previous = try env.setWorkers(0)
        defer { _ = try? env.setWorkers(previous == 0 ? 2 : previous) }

        let app = XCUIApplication()
        app.launchArguments = [
            "-paired-test", env.host, "\(env.port)", token,
            "-auto-submit-image", "sample.jpg,sample.jpg",
        ]
        app.launch()
        defer { attachShot(app, name: "cancel-queued") }

        let rows = app.descendants(matching: .any).matching(identifier: "job-row")
        let deadline = Date().addingTimeInterval(25)
        while Date() < deadline, rows.count < 2 {
            RunLoop.current.run(until: Date().addingTimeInterval(0.4))
        }
        XCTAssertEqual(rows.count, 2, "two queued jobs; banner=\(bannerLabel(app))")

        XCTAssertTrue(
            wait(for: {
                self.rowValues(app).filter { $0.contains("排队中") }.count >= 2
            }, timeout: 10),
            "both rows queued before cancel; \(rowValues(app))"
        )

        let cancel = app.buttons["job-cancel"].firstMatch.exists
            ? app.buttons["job-cancel"].firstMatch
            : app.buttons["取消"].firstMatch
        XCTAssertTrue(cancel.waitForExistence(timeout: 5), "cancel on queued row")
        cancel.tap()

        XCTAssertTrue(
            wait(for: {
                let labels = self.rowValues(app)
                return labels.contains(where: { $0.contains("已取消") })
                    && labels.contains(where: { $0.contains("排队中") })
            }, timeout: 15),
            "one cancelled, one still queued; \(rowValues(app))"
        )
        attachShot(app, name: "cancel-queued-done")
    }

    func testConfigWebViewLoads() throws {
        let env = try UITestEnv.load()
        let token = try env.tokenOrPair()
        let app = XCUIApplication()
        app.launchArguments = ["-paired-test", env.host, "\(env.port)", token]
        app.launch()

        XCTAssertTrue(app.tabBars.buttons["配置"].waitForExistence(timeout: 10))
        app.tabBars.buttons["配置"].tap()
        XCTAssertTrue(app.navigationBars["配置"].waitForExistence(timeout: 10))
        _ = app.descendants(matching: .any)["config-webview"].waitForExistence(timeout: 10)
        XCTAssertFalse(app.descendants(matching: .any)["engine-banner"].exists)
        attachShot(app, name: "config-webview")
    }

    private func flipForward(_ app: XCUIApplication, status: XCUIElement) {
        let before = status.label
        app.typeKey(XCUIKeyboardKey.rightArrow, modifierFlags: [])
        if status.label == before {
            app.buttons["reader-next"].tap()
        }
    }

    private func pressSpace(_ app: XCUIApplication) {
        let space = app.buttons["reader-space"]
        if space.waitForExistence(timeout: 2) {
            space.tap()
            return
        }
        app.typeKey(XCUIKeyboardKey.space, modifierFlags: [])
    }

    private func wait(for element: XCUIElement, containing needle: String, timeout: TimeInterval) -> Bool {
        wait(for: element, matching: { $0.contains(needle) }, timeout: timeout)
    }

    private func wait(for predicate: @escaping () -> Bool, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if predicate() {
                return true
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.4))
        }
        return predicate()
    }

    private func wait(for element: XCUIElement, matching predicate: @escaping (String) -> Bool, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if predicate(element.label) {
                return true
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.4))
        }
        return predicate(element.label)
    }

    private func bannerLabel(_ app: XCUIApplication) -> String {
        let banner = app.staticTexts["engine-banner"]
        return banner.exists ? banner.label : ""
    }

    private func rowValues(_ app: XCUIApplication) -> [String] {
        let rows = app.descendants(matching: .any).matching(identifier: "job-row")
        return (0..<rows.count).map { i in
            let row = rows.element(boundBy: i)
            return (row.value as? String) ?? row.label
        }
    }

    private func attachShot(_ app: XCUIApplication, name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}

struct UITestEnv {
    var host: String
    var port: UInt16
    var pairingCodeFile: String
    var token: String?
    var batchFolder: String?

    static func load() throws -> UITestEnv {
        let path = "/tmp/pr-client-uitest.env"
        guard FileManager.default.fileExists(atPath: path),
              let text = try? String(contentsOfFile: path, encoding: .utf8)
        else {
            throw XCTSkip("no /tmp/pr-client-uitest.env")
        }
        var values: [String: String] = [:]
        for line in text.split(whereSeparator: \.isNewline) {
            let parts = line.split(separator: "=", maxSplits: 1)
            guard parts.count == 2 else { continue }
            values[String(parts[0]).trimmingCharacters(in: .whitespaces)] =
                String(parts[1]).trimmingCharacters(in: .whitespaces)
        }
        guard let host = values["DAEMON_HOST"],
              let portRaw = values["DAEMON_PORT"],
              let port = UInt16(portRaw),
              let codeFile = values["PAIRING_CODE_FILE"]
        else {
            throw XCTSkip("incomplete /tmp/pr-client-uitest.env")
        }
        let token = values["DAEMON_TOKEN"]
        let batch = values["BATCH_FOLDER"]
        return UITestEnv(host: host, port: port, pairingCodeFile: codeFile, token: token, batchFolder: batch)
    }

    func setWorkers(_ workers: Int) throws -> Int {
        let token = try tokenOrPair()
        let get = URL(string: "http://\(host):\(port)/config")!
        var request = URLRequest(url: get)
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        let data = try sync(request)
        guard var obj = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw XCTSkip("config was not an object")
        }
        let previous = (obj["workers"] as? Int) ?? (obj["workers"] as? NSNumber)?.intValue ?? 2
        obj["workers"] = workers
        var put = URLRequest(url: get)
        put.httpMethod = "PUT"
        put.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        put.setValue("application/json", forHTTPHeaderField: "Content-Type")
        put.httpBody = try JSONSerialization.data(withJSONObject: obj)
        _ = try sync(put)
        var restart = URLRequest(url: URL(string: "http://\(host):\(port)/engine/restart")!)
        restart.httpMethod = "POST"
        restart.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        _ = try sync(restart)
        return previous
    }

    private func sync(_ request: URLRequest) throws -> Data {
        let sem = DispatchSemaphore(value: 0)
        var data = Data()
        var status = 0
        URLSession.shared.dataTask(with: request) { body, response, _ in
            data = body ?? Data()
            status = (response as? HTTPURLResponse)?.statusCode ?? 0
            sem.signal()
        }.resume()
        XCTAssertEqual(sem.wait(timeout: .now() + 30), .success)
        XCTAssertTrue((200...299).contains(status), "http \(status) \(request.url?.path ?? "")")
        return data
    }

    func tokenOrPair() throws -> String {
        if let token, !token.isEmpty {
            return token
        }
        try pairRequest()
        let code = try waitForPairingCode(path: pairingCodeFile, timeout: 30)
        return try confirm(code: code)
    }

    private func pairRequest() throws {
        let url = URL(string: "http://\(host):\(port)/pair/request")!
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        let sem = DispatchSemaphore(value: 0)
        var ok = false
        URLSession.shared.dataTask(with: request) { _, response, _ in
            ok = (response as? HTTPURLResponse)?.statusCode == 200
            sem.signal()
        }.resume()
        XCTAssertEqual(sem.wait(timeout: .now() + 10), .success)
        XCTAssertTrue(ok, "POST /pair/request")
    }

    private func confirm(code: String) throws -> String {
        let url = URL(string: "http://\(host):\(port)/pair/confirm")!
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: ["code": code])
        let sem = DispatchSemaphore(value: 0)
        var token: String?
        URLSession.shared.dataTask(with: request) { data, _, _ in
            if let data,
               let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            {
                token = obj["token"] as? String
            }
            sem.signal()
        }.resume()
        XCTAssertEqual(sem.wait(timeout: .now() + 10), .success)
        guard let token, !token.isEmpty else {
            throw XCTSkip("pair confirm returned no token")
        }
        return token
    }
}

func waitForPairingCode(path: String, timeout: TimeInterval) throws -> String {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
        if let text = try? String(contentsOfFile: path, encoding: .utf8) {
            let digits = text.filter(\.isNumber)
            if digits.count >= 6 {
                return String(digits.prefix(6))
            }
        }
        RunLoop.current.run(until: Date().addingTimeInterval(0.2))
    }
    throw XCTSkip("pairing code file stayed empty")
}
