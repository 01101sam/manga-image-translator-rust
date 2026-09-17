import XCTest
@testable import ImageTranslator

final class PageMachineTests: XCTestCase {
    func testSpaceOnUntranslatedEnqueuesAndLeavesState() {
        let page = ReaderPage(index: 0, translation: .untranslated, viewing: .original, jobId: nil)
        let (next, command) = reducePage(page, .space)
        XCTAssertEqual(next, page)
        XCTAssertEqual(command, .enqueue)
    }

    func testSubmittedThenRunningThenDone() {
        var page = ReaderPage(index: 1, translation: .untranslated, viewing: .original, jobId: nil)
        (page, _) = reducePage(page, .submitted(jobId: "job-1"))
        XCTAssertEqual(page, ReaderPage(index: 1, translation: .queued, viewing: .original, jobId: "job-1"))
        (page, _) = reducePage(page, .jobState(.running))
        XCTAssertEqual(page.translation, .running)
        (page, _) = reducePage(page, .jobState(.done))
        XCTAssertEqual(page, ReaderPage(index: 1, translation: .done, viewing: .original, jobId: "job-1"))
    }

    func testSpaceOnDoneTogglesTranslatedThenOriginal() {
        var page = ReaderPage(index: 2, translation: .done, viewing: .original, jobId: "job-2")
        var command: PageCommand
        (page, command) = reducePage(page, .space)
        XCTAssertEqual(command, .toggleView)
        XCTAssertEqual(page.viewing, .translated)
        XCTAssertEqual(page.translation, .done)
        (page, command) = reducePage(page, .space)
        XCTAssertEqual(command, .toggleView)
        XCTAssertEqual(page.viewing, .original)
    }

    func testSpaceOnQueuedAndRunningDoesNothing() {
        let queued = ReaderPage(index: 0, translation: .queued, viewing: .original, jobId: "j")
        let running = ReaderPage(index: 0, translation: .running, viewing: .original, jobId: "j")
        XCTAssertEqual(reducePage(queued, .space).1, .none)
        XCTAssertEqual(reducePage(queued, .space).0, queued)
        XCTAssertEqual(reducePage(running, .space).1, .none)
        XCTAssertEqual(reducePage(running, .space).0, running)
    }

    func testFailedAndCancelledReturnToUntranslated() {
        let page = ReaderPage(index: 3, translation: .running, viewing: .original, jobId: "j3")
        XCTAssertEqual(
            reducePage(page, .jobState(.failed)).0,
            ReaderPage(index: 3, translation: .untranslated, viewing: .original, jobId: nil)
        )
        XCTAssertEqual(
            reducePage(page, .jobState(.cancelled)).0,
            ReaderPage(index: 3, translation: .untranslated, viewing: .original, jobId: nil)
        )
    }

    func testTranslateAllSelectsOnlyUntranslatedIndexes() {
        let pages = [
            ReaderPage(index: 0, translation: .done, viewing: .original, jobId: "a"),
            ReaderPage(index: 1, translation: .untranslated, viewing: .original, jobId: nil),
            ReaderPage(index: 2, translation: .queued, viewing: .original, jobId: "b"),
            ReaderPage(index: 3, translation: .untranslated, viewing: .original, jobId: nil),
        ]
        XCTAssertEqual(untranslatedIndexes(pages), [1, 3])
    }
}
