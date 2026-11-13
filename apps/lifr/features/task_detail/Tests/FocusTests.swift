import ComposableArchitecture
import Foundation
import LifrCore
import LifrGlance
import LifrGlanceClient
import LifrLockClient
import LifrRemindersClient
import LifrStoreClient
import LifrTaskDetail
import XCTest

@MainActor
final class FocusTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")

    private func makeStore(
        _ task: LifrTask, focus: FocusActivityClient
    ) throws -> TestStoreOf<TaskDetailFeature> {
        let state = try XCTUnwrap(
            TaskDetailFeature.State(id: task.id, workspace: records.workspace, now: now, timeZone: .gmt))
        let store = TestStore(initialState: state) {
            TaskDetailFeature()
        } withDependencies: {
            $0.store = .inMemory(records.workspace)
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = .incrementing
            $0.focusActivity = focus
            $0.reminders = RemindersClient.inMemory().client
        }
        store.exhaustivity = .off
        return store
    }

    func testATaskInProgressGoesOnTheLockScreenAndComesOff() async throws {
        let stretch = records.task("Stretch", in: health, status: .inProgress)
        let (focus, showing) = FocusActivityClient.inMemory()
        let store = try makeStore(stretch, focus: focus)

        await store.send(.appeared)
        await store.receive(\.focusChecked) {
            $0.canFocus = true
            $0.isFocusing = false
        }
        await store.send(.focusTapped)
        await store.receive(\.focusChecked) { $0.isFocusing = true }
        XCTAssertEqual(showing.value[stretch.id]?.title, "Stretch")

        await store.send(.focusTapped)
        await store.receive(\.focusChecked) { $0.isFocusing = false }
        XCTAssertNil(showing.value[stretch.id])
    }

    func testWhileTheLockIsOnTheLockScreenShowsNoTitle() async throws {
        @Shared(.locksApp) var locked
        $locked.withLock { $0 = true }
        let stretch = records.task("Stretch", in: health, status: .inProgress)
        let (focus, showing) = FocusActivityClient.inMemory()
        let store = try makeStore(stretch, focus: focus)

        await store.send(.focusTapped)
        await store.receive(\.focusChecked)
        XCTAssertEqual(showing.value[stretch.id]?.title, "")
    }

    func testWithLiveActivitiesOffItSaysHowToTurnThemOn() async throws {
        let stretch = records.task("Stretch", in: health, status: .inProgress)
        let (focus, showing) = FocusActivityClient.inMemory(available: false)
        let store = try makeStore(stretch, focus: focus)

        await store.send(.appeared)
        XCTAssertFalse(store.state.canFocus)
        await store.send(.focusTapped)
        await store.receive(\.focusFailed)
        XCTAssertNotNil(store.state.alert)
        XCTAssertTrue(showing.value.isEmpty)
    }
}
