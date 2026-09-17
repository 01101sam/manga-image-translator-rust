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
}
