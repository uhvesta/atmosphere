import ComposableArchitecture
import Foundation
import LifrCore
import LifrProcrastinated
import LifrStoreClient
import XCTest

/// Flow 4, the weekly Procrastinated review.
@MainActor
final class ProcrastinatedFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")

    private func load() async -> TestStoreOf<ProcrastinatedFeature> {
        let store = TestStore(initialState: ProcrastinatedFeature.State()) {
            ProcrastinatedFeature()
        } withDependencies: { [now, records] in
            $0.store = .inMemory(records.workspace)
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        await store.send(.list(.task))
        await store.receive(\.list.loaded)
        return store
    }

    func testListsStalledTasksOldestActivityFirstWithWhy() async {
        let byHand = records.task("Fix the bike", in: health, status: .procrastinated, idle: 3)
        let idle = records.task("Plan long runs", in: health, status: .inProgress, idle: 12)
        records.task("Stretch", in: health, status: .inProgress, idle: 6)
        records.task("Someday", in: health, idle: 30)

        let store = await load()

        XCTAssertEqual(store.state.list.rows.map(\.title), ["Plan long runs", "Fix the bike"])
        let notes = store.state.list.sections.first?.notes
        XCTAssertEqual(notes?[idle.id], Procrastination.inactive(days: 12).note)
        XCTAssertEqual(notes?[byHand.id], Procrastination.setByHand.note)
    }

    func testRestartTakesItOffTheList() async {
        let plan = records.task("Plan long runs", in: health, status: .inProgress, idle: 12)
        let store = await load()

        await store.send(.list(.rowAction(plan.id, .status(.inProgress))))
        await store.receive(\.list.delegate.saved)

        XCTAssertTrue(store.state.list.isEmpty)
    }

    func testRescheduleClearsAnAutomaticOneButNotOneSetByHand() async throws {
        let plan = records.task("Plan long runs", in: health, status: .inProgress, idle: 12)
        let bike = records.task("Fix the bike", in: health, status: .procrastinated, idle: 3)
        let store = await load()

        for task in [plan, bike] {
            await store.send(.list(.rowAction(task.id, .reschedule)))
            await store.send(.list(.reschedule(.presented(.saveTapped))))
            await store.receive(\.list.delegate.saved)
        }

        XCTAssertEqual(store.state.list.rows.map(\.title), ["Fix the bike"])
        XCTAssertEqual(store.state.list.workspace?.tasks[bike.id]?.dueDate, try CalendarDate("2026-11-04"))
    }

    func testWontDoWithOpenSubtasksAsksFirst() async {
        let plan = records.task("Plan long runs", in: health, status: .procrastinated)
        records.task("Pick a race", in: health, parent: plan)
        let store = await load()

        await store.send(.list(.rowAction(plan.id, .status(.wontDo))))
        await store.receive(\.list.saved)
        XCTAssertNotNil(store.state.list.prompt)

        await store.send(.list(.prompt(.presented(.dropWithSubtasks(task: plan.id)))))
        await store.receive(\.list.delegate.saved)

        XCTAssertTrue(store.state.list.isEmpty)
    }
}
