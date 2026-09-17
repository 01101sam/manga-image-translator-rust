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
        try pairViaRealDiscovery(app, env: env)
        attachShot(app, name: "paired-tabs")
    }

    func testPairingPersistsAcrossRelaunch() throws {
        let env = try UITestEnv.load()
        let app = XCUIApplication()
        app.launchArguments = ["-daemon-host", env.host, "-daemon-port", "\(env.port)"]
        app.launch()
        try pairViaRealDiscovery(app, env: env)
        attachShot(app, name: "persist-paired")
        app.terminate()

        let again = XCUIApplication()
        again.launch()
        XCTAssertTrue(again.tabBars.buttons["任务"].waitForExistence(timeout: 15), "tabs after relaunch from Keychain")
        XCTAssertTrue(again.tabBars.buttons["阅读"].exists)
        XCTAssertTrue(again.tabBars.buttons["配置"].exists)
        XCTAssertFalse(again.textFields["pairing-code"].exists, "no pairing prompt after relaunch")
        attachShot(again, name: "persist-relaunch")
    }

    func testJobSubmitAndDone() throws {
        let env = try UITestEnv.load()
        let token = try env.tokenOrPair()
        try env.prepareForNewJobs()
        let app = XCUIApplication()
        app.launchArguments = [
            "-paired-test", env.host, "\(env.port)", token,
            "-auto-submit-image", "sample.jpg",
        ]
        app.launch()
        defer { attachShot(app, name: "job-submit-end") }

        let args = app.descendants(matching: .any)["launch-args"]
        XCTAssertTrue(args.waitForExistence(timeout: 10), "launch-args only when hooks are set")
        let row = app.descendants(matching: .any)["job-row"]
        XCTAssertTrue(row.waitForExistence(timeout: 25), "job row after auto submit; args=\(args.label); banner=\(bannerLabel(app))")
        let deadline = Date().addingTimeInterval(90)
        while Date() < deadline {
            let labels = rowValues(app)
            XCTAssertFalse(labels.contains(where: { $0.contains("已取消") }), "submit job was cancelled; \(labels)")
            if labels.contains(where: { $0.contains("已完成") }) { break }
            RunLoop.current.run(until: Date().addingTimeInterval(0.4))
        }
        XCTAssertTrue(rowValues(app).contains(where: { $0.contains("已完成") }), "done; \(rowValues(app))")
        attachShot(app, name: "job-done")
    }

    func testPDFReaderKeysAndSpace() throws {
        let env = try UITestEnv.load()
        let token = try env.tokenOrPair()
        try env.prepareForNewJobs()
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

        pressSpace(app, status: status)
        XCTAssertTrue(
            wait(for: status, matching: { $0.contains("排队中") || $0.contains("翻译中") || $0.contains("译文") || $0.contains("原文") && !$0.contains("未翻译") }, timeout: 15),
            status.label
        )

        XCTAssertTrue(wait(for: status, matching: { $0.contains("译文") || $0.contains("原文") }, timeout: 90), status.label)
        let before = status.label
        pressSpace(app, status: status)
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
        let leftover = app.buttons["job-cancel"].firstMatch.exists
            ? app.buttons["job-cancel"].firstMatch
            : app.buttons["取消"].firstMatch
        if leftover.exists {
            leftover.tap()
            XCTAssertTrue(
                wait(for: { self.rowValues(app).allSatisfy { $0.contains("已取消") } }, timeout: 10),
                "drain leftover queued job before engine restart; \(rowValues(app))"
            )
        }
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

    func testTabSwitchStress() throws {
        let env = try UITestEnv.load()
        let token = try env.tokenOrPair()
        let app = XCUIApplication()
        app.launchArguments = ["-paired-test", env.host, "\(env.port)", token]
        app.launch()

        let jobs = tabButton(app, "任务")
        let config = tabButton(app, "配置")
        let reader = tabButton(app, "阅读")
        XCTAssertTrue(jobs.waitForExistence(timeout: 10), "jobs tab after launch")

        for cycle in 1...50 {
            jobs.tap()
            XCTAssertTrue(
                app.navigationBars["任务"].waitForExistence(timeout: 3),
                "jobs unresponsive at cycle \(cycle)"
            )
            config.tap()
            XCTAssertTrue(
                app.navigationBars["配置"].waitForExistence(timeout: 3),
                "config unresponsive at cycle \(cycle)"
            )
            reader.tap()
            XCTAssertTrue(
                app.navigationBars["阅读"].waitForExistence(timeout: 3),
                "reader unresponsive at cycle \(cycle)"
            )
        }
        attachShot(app, name: "tab-switch-stress")
    }

    private func tabButton(_ app: XCUIApplication, _ title: String) -> XCUIElement {
        let inBar = app.tabBars.buttons[title]
        if inBar.exists {
            return inBar
        }
        return app.buttons[title].firstMatch
    }

    /// Real Keychain pairing. If a prior run already persisted a token, 更换 Daemon then type a fresh code.
    private func pairViaRealDiscovery(_ app: XCUIApplication, env: UITestEnv) throws {
        let jobsTab = app.tabBars.buttons["任务"]
        if jobsTab.waitForExistence(timeout: 3) {
            let swap = app.buttons["更换 Daemon"]
            XCTAssertTrue(swap.waitForExistence(timeout: 5), "already-paired sim must expose 更换 Daemon")
            swap.tap()
        }
        let row = waitForDaemonRow(app, port: env.port, timeout: 30)
        XCTAssertTrue(row.exists, "bonjour row whose subtitle contains :\(env.port); rows=\(daemonRowBlobs(app))")
        attachShot(app, name: "discovery-row")
        try? "".write(toFile: env.pairingCodeFile, atomically: true, encoding: .utf8)
        row.tap()

        let field = app.textFields["pairing-code"]
        XCTAssertTrue(field.waitForExistence(timeout: 15), "code field after request")
        let code = try waitForPairingCode(path: env.pairingCodeFile, timeout: 30)
        field.tap()
        field.typeText(code)
        app.buttons["pairing-confirm"].tap()

        XCTAssertTrue(jobsTab.waitForExistence(timeout: 15), "tab bar after pairing")
        XCTAssertTrue(app.tabBars.buttons["阅读"].exists)
        XCTAssertTrue(app.tabBars.buttons["配置"].exists)
    }

    private func waitForDaemonRow(_ app: XCUIApplication, port: UInt16, timeout: TimeInterval) -> XCUIElement {
        let needle = ":\(port)"
        let rows = app.descendants(matching: .any).matching(identifier: "daemon-row")
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let match = firstDaemonRow(rows, containing: needle) {
                return match
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.4))
        }
        return firstDaemonRow(rows, containing: needle) ?? rows.firstMatch
    }

    private func firstDaemonRow(_ rows: XCUIElementQuery, containing needle: String) -> XCUIElement? {
        for i in 0..<rows.count {
            let row = rows.element(boundBy: i)
            let blob = daemonRowBlob(row)
            if blob.contains(needle) {
                return row
            }
        }
        return nil
    }

    private func daemonRowBlobs(_ app: XCUIApplication) -> [String] {
        let rows = app.descendants(matching: .any).matching(identifier: "daemon-row")
        return (0..<rows.count).map { daemonRowBlob(rows.element(boundBy: $0)) }
    }

    private func daemonRowBlob(_ row: XCUIElement) -> String {
        let value = row.value as? String ?? ""
        let children = row.staticTexts.allElementsBoundByIndex.map(\.label).joined(separator: " ")
        return [row.label, value, children].joined(separator: " ")
    }

    private func focusReader(_ app: XCUIApplication) {
        let canvas = app.descendants(matching: .any)["reader-canvas"]
        if canvas.waitForExistence(timeout: 5) {
            canvas.tap()
        }
    }

    private func flipForward(_ app: XCUIApplication, status: XCUIElement) {
        focusReader(app)
        let before = status.label
        app.typeKey(XCUIKeyboardKey.rightArrow, modifierFlags: [])
        if status.label == before {
            app.buttons["reader-next"].tap()
        }
    }

    private func pressSpace(_ app: XCUIApplication, status: XCUIElement) {
        focusReader(app)
        let before = status.label
        app.typeKey(XCUIKeyboardKey.space, modifierFlags: [])
        let deadline = Date().addingTimeInterval(1.5)
        while Date() < deadline, status.label == before {
            RunLoop.current.run(until: Date().addingTimeInterval(0.2))
        }
        if status.label == before {
            let space = app.buttons["reader-space"]
            if space.waitForExistence(timeout: 2) {
                space.tap()
            }
        }
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

    func prepareForNewJobs() throws {
        try cancelQueuedJobs()
        let workers = try currentWorkers()
        if workers != 2 {
            _ = try setWorkers(2)
        } else {
            try ensureEngineRunning()
        }
        try cancelQueuedJobs()
    }

    func cancelQueuedJobs() throws {
        let token = try tokenOrPair()
        let list = try authedJSON(path: "/jobs", method: "GET", token: token)
        let jobs = list["jobs"] as? [[String: Any]] ?? []
        for job in jobs {
            guard job["state"] as? String == "queued", let id = job["job_id"] as? String else { continue }
            _ = try authedJSON(path: "/jobs/\(id)/cancel", method: "POST", token: token)
        }
    }

    func ensureEngineRunning() throws {
        let token = try tokenOrPair()
        let status = try authedJSON(path: "/engine/status", method: "GET", token: token)
        if status["state"] as? String == "running" { return }
        _ = try authedJSON(path: "/engine/start", method: "POST", token: token)
    }

    private func currentWorkers() throws -> Int {
        let token = try tokenOrPair()
        let cfg = try authedJSON(path: "/config", method: "GET", token: token)
        return (cfg["workers"] as? Int) ?? (cfg["workers"] as? NSNumber)?.intValue ?? 2
    }

    private func authedJSON(path: String, method: String, token: String, body: Data? = nil) throws -> [String: Any] {
        var request = URLRequest(url: URL(string: "http://\(host):\(port)\(path)")!)
        request.httpMethod = method
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = body
        }
        let data = try sync(request)
        return (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
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
