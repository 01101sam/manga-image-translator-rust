import XCTest
@testable import ImageTranslator

final class JobMachineTests: XCTestCase {
    func testQueuedToRunningToDone() {
        var row = JobRow(
            snapshot: JobSnapshot(jobId: "j1", state: .queued, positionInQueue: 2, error: nil),
            cancelPending: false
        )
        var command: JobCommand
        (row, command) = reduceJob(
            row,
            .snapshot(JobSnapshot(jobId: "j1", state: .running, positionInQueue: nil, error: nil))
        )
        XCTAssertEqual(command, .none)
        XCTAssertEqual(row.snapshot.state, .running)
        XCTAssertNil(row.snapshot.positionInQueue)
        (row, command) = reduceJob(
            row,
            .snapshot(JobSnapshot(jobId: "j1", state: .done, positionInQueue: nil, error: nil))
        )
        XCTAssertEqual(row.snapshot, JobSnapshot(jobId: "j1", state: .done, positionInQueue: nil, error: nil))
    }

    func testCancelOnlyFromQueued() {
        let queued = JobRow(
            snapshot: JobSnapshot(jobId: "j2", state: .queued, positionInQueue: 1, error: nil),
            cancelPending: false
        )
        let (pending, command) = reduceJob(queued, .requestCancel)
        XCTAssertEqual(command, .cancel(jobId: "j2"))
        XCTAssertTrue(pending.cancelPending)

        for state: JobLifecycle in [.running, .done, .failed, .cancelled] {
            let row = JobRow(
                snapshot: JobSnapshot(jobId: "j2", state: state, positionInQueue: nil, error: nil),
                cancelPending: false
            )
            let (next, cmd) = reduceJob(row, .requestCancel)
            XCTAssertEqual(cmd, .none)
            XCTAssertEqual(next, row)
        }
    }

    func testCancelAcceptedMarksCancelled() {
        let row = JobRow(
            snapshot: JobSnapshot(jobId: "j3", state: .queued, positionInQueue: 3, error: nil),
            cancelPending: true
        )
        let (next, command) = reduceJob(row, .cancelAccepted)
        XCTAssertEqual(command, .none)
        XCTAssertEqual(next.snapshot.state, .cancelled)
        XCTAssertNil(next.snapshot.positionInQueue)
        XCTAssertFalse(next.cancelPending)
    }

    func testCancelRejectedClearsPending() {
        let row = JobRow(
            snapshot: JobSnapshot(jobId: "j4", state: .queued, positionInQueue: 1, error: nil),
            cancelPending: true
        )
        let (next, _) = reduceJob(row, .cancelRejected)
        XCTAssertEqual(next.snapshot.state, .queued)
        XCTAssertFalse(next.cancelPending)
    }

    func testSecondCancelWhilePendingIsIgnored() {
        let row = JobRow(
            snapshot: JobSnapshot(jobId: "j5", state: .queued, positionInQueue: 1, error: nil),
            cancelPending: true
        )
        let (next, command) = reduceJob(row, .requestCancel)
        XCTAssertEqual(command, .none)
        XCTAssertEqual(next, row)
    }

    func testShortJobIdUsesFirstUUIDSegment() {
        XCTAssertEqual(shortJobId("a1b2c3d4-e5f6-7890-abcd-ef1234567890"), "a1b2c3d4")
        XCTAssertEqual(shortJobId("solo"), "solo")
    }

    func testReduceJobKeepsThumbnail() {
        let thumb = Data([9, 8, 7])
        var row = JobRow(
            snapshot: JobSnapshot(jobId: "j-thumb", state: .queued, positionInQueue: 1, error: nil),
            cancelPending: false,
            thumbnail: thumb
        )
        (row, _) = reduceJob(
            row,
            .snapshot(JobSnapshot(jobId: "j-thumb", state: .running, positionInQueue: nil, error: nil))
        )
        XCTAssertEqual(row.thumbnail, thumb)
    }

    func testMoveJobHighlightFromNilSelectsEdge() {
        XCTAssertEqual(moveJobHighlight(count: 3, current: nil, delta: 1), 0)
        XCTAssertEqual(moveJobHighlight(count: 3, current: nil, delta: -1), 2)
        XCTAssertNil(moveJobHighlight(count: 0, current: nil, delta: 1))
    }

    func testMoveJobHighlightClamps() {
        XCTAssertEqual(moveJobHighlight(count: 3, current: 0, delta: -1), 0)
        XCTAssertEqual(moveJobHighlight(count: 3, current: 2, delta: 1), 2)
        XCTAssertEqual(moveJobHighlight(count: 3, current: 1, delta: 1), 2)
        XCTAssertEqual(moveJobHighlight(count: 3, current: 1, delta: -1), 0)
    }

    func testAdjacentDoneJobSkipsActiveRows() {
        let jobs = [
            job("a", .queued),
            job("b", .done),
            job("c", .running),
            job("d", .done),
        ]
        XCTAssertEqual(adjacentDoneJobId(jobs: jobs, currentId: "b", delta: 1), "d")
        XCTAssertEqual(adjacentDoneJobId(jobs: jobs, currentId: "d", delta: -1), "b")
        XCTAssertEqual(adjacentDoneJobId(jobs: jobs, currentId: "d", delta: 1), "d")
        XCTAssertEqual(adjacentDoneJobId(jobs: jobs, currentId: "c", delta: 1), "b")
    }

    func testVerticalArrowsAlwaysMoveSelection() {
        XCTAssertEqual(jobListArrowDelta(.up, previewOpen: false), -1)
        XCTAssertEqual(jobListArrowDelta(.down, previewOpen: false), 1)
        XCTAssertEqual(jobListArrowDelta(.up, previewOpen: true), -1)
        XCTAssertEqual(jobListArrowDelta(.down, previewOpen: true), 1)
    }

    func testHorizontalArrowsAliasVerticalOnlyWhenPreviewOpen() {
        XCTAssertNil(jobListArrowDelta(.left, previewOpen: false))
        XCTAssertNil(jobListArrowDelta(.right, previewOpen: false))
        XCTAssertEqual(jobListArrowDelta(.left, previewOpen: true), -1)
        XCTAssertEqual(jobListArrowDelta(.right, previewOpen: true), 1)
    }

    func testEnterOpensPreviewOnlyWhenDone() {
        XCTAssertTrue(canOpenJobPreview(.done))
        XCTAssertFalse(canOpenJobPreview(.queued))
        XCTAssertFalse(canOpenJobPreview(.running))
        XCTAssertFalse(canOpenJobPreview(.failed))
    }

    func testLocalRemoveDropsRowAndArtifact() {
        var jobs = [job("keep", .done), job("gone", .done)]
        var artifacts = ["keep": Data([1]), "gone": Data([2])]
        removeLocalJobs(jobs: &jobs, artifacts: &artifacts, at: IndexSet(integer: 1))
        XCTAssertEqual(jobs.map(\.snapshot.jobId), ["keep"])
        XCTAssertEqual(Array(artifacts.keys), ["keep"])
    }

    func testLocalMoveReorders() {
        var jobs = [job("a", .queued), job("b", .queued), job("c", .queued)]
        moveLocalJobs(jobs: &jobs, from: IndexSet(integer: 0), to: 3)
        XCTAssertEqual(jobs.map(\.snapshot.jobId), ["b", "c", "a"])
    }

    private func job(_ id: String, _ state: JobLifecycle) -> JobRow {
        JobRow(
            snapshot: JobSnapshot(jobId: id, state: state, positionInQueue: nil, error: nil),
            cancelPending: false
        )
    }
}
