import ComposableArchitecture
import Foundation
import XCTest

@testable import LifrBugReport

final class BugRecorderTests: XCTestCase {
    private struct Screen: Equatable {
        var title = "Inbox"
        var count = 0
    }

    func testARecordingHasTheStartEachActionWithItsTimeAndWhatItChanged() {
        let clock = LockIsolated(Date(timeIntervalSince1970: 100))
        let recorder = BugRecorder(now: { clock.value })

        recorder.start(from: Screen())
        clock.setValue(Date(timeIntervalSince1970: 102.5))
        recorder.log("increment", from: Screen(), to: Screen(count: 1))
        clock.setValue(Date(timeIntervalSince1970: 103))
        recorder.log("refresh", from: Screen(count: 1), to: Screen(count: 1))
        let recording = recorder.stop()

        XCTAssertEqual(recording?.startedAt, Date(timeIntervalSince1970: 100))
        XCTAssertTrue(recording?.start.contains("Inbox") ?? false)
        XCTAssertEqual(recording?.steps.map(\.offset), [2.5, 3])
        XCTAssertTrue(recording?.steps.first?.changes?.contains("+") ?? false)
        XCTAssertNil(recording?.steps.last?.changes)
        XCTAssertFalse(recorder.isRecording)
    }

    func testNothingIsLoggedWhenNotRecording() {
        let recorder = BugRecorder()

        recorder.log("increment", from: Screen(), to: Screen(count: 1))

        XCTAssertNil(recorder.stop())
    }

    func testAFullRecordingCountsTheStepsItLeavesOut() {
        let recorder = BugRecorder()
        recorder.start(from: Screen())

        for index in 0..<(BugRecorder.stepLimit + 5) {
            recorder.log(index, from: Screen(), to: Screen())
        }

        let recording = recorder.stop()
        XCTAssertEqual(recording?.steps.count, BugRecorder.stepLimit)
        XCTAssertEqual(recording?.dropped, 5)
    }

    func testTheScreenIsTakenOnceItSettlesAfterAChange() async {
        let clock = TestClock()
        let recorder = BugRecorder(capture: { Data([7]) }, clock: clock)
        recorder.start(from: Screen())
        await settle(clock)
        recorder.log("increment", from: Screen(), to: Screen(count: 1))
        recorder.log("increment", from: Screen(count: 1), to: Screen(count: 2))
        await settle(clock)

        let recording = recorder.stop()

        XCTAssertEqual(recording?.screenshots.map(\.afterSteps), [0, 2])
    }

    /// Lets the screenshot's task start waiting, passes the wait, then lets
    /// it finish.
    private func settle(_ clock: TestClock<Duration>) async {
        await Task.megaYield()
        await clock.advance(by: .seconds(1))
        await Task.megaYield()
    }

    func testTheWrapperLogsEachActionWhileRecording() {
        let recorder = BugRecorder()
        let reducer = withDependencies {
            $0.bugRecorder = recorder
        } operation: {
            Reduce<Int, String> { state, _ in
                state += 1
                return .none
            }
            .recordingBugs()
        }
        var state = 0

        _ = reducer.reduce(into: &state, action: "before")
        recorder.start(from: state)
        _ = reducer.reduce(into: &state, action: "during")

        XCTAssertEqual(state, 2)
        XCTAssertEqual(recorder.stop()?.steps.count, 1)
    }
}
