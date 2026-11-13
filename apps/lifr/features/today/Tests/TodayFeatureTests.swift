import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskList
import LifrToday
import XCTest

@MainActor
final class TodayFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")

    private func load() async -> TestStoreOf<TodayFeature> {
        let store = TestStore(initialState: TodayFeature.State()) {
            TodayFeature()
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

    private func titles(_ store: TestStoreOf<TodayFeature>, _ section: String) -> [String] {
        store.state.list.sections.first { $0.id == section }?.rows.map(\.title) ?? []
    }

    func testAnEmptyTodaySaysTheDayIsDoneOnlyOnceSomethingClosedToday() async {
        records.task("Closed last week", in: health, status: .done, idle: 7)
        var store = await load()
        XCTAssertEqual(store.state.list.emptyMessage, TaskQuery.today.empty)

        records.task("Stretch", in: health, status: .done, due: 0)
        store = await load()
        XCTAssertTrue(store.state.list.isEmpty)
        XCTAssertNotEqual(store.state.list.emptyMessage, TaskQuery.today.empty)
        XCTAssertEqual(store.state.list.emptyMessage, TaskQuery.today.finished)
    }

    func testAnEmptyTodayOffersAnytimeOnlyWhenItHasTasks() async {
        records.task("Due tomorrow", in: health, due: 1)
        var store = await load()
        XCTAssertFalse(store.state.canPlanFromAnytime)

        records.task("Someday", in: health)
        store = await load()
        XCTAssertTrue(store.state.canPlanFromAnytime)
        await store.send(.planFromAnytimeTapped)
        await store.receive(\.delegate.planFromAnytime)

        records.task("Stretch", in: health, due: 0)
        store = await load()
        XCTAssertFalse(store.state.canPlanFromAnytime)
    }

    func testShowsOverdueAndDueTodayOpenTasksOnly() async {
        records.task("Stretch", in: health, due: 0)
        records.task("Book physio", in: health, due: -2)
        records.task("Renew passport", in: health, due: 3)
        records.task("Someday", in: health)
        records.task("Already done", in: health, status: .done, due: -1)

        let store = await load()

        XCTAssertEqual(titles(store, "overdue"), ["Book physio"])
        XCTAssertEqual(titles(store, "today"), ["Stretch"])
    }

    func testDueTimeEarlierTodayIsOverdue() async {
        records.task("Before now", in: health, due: 0, at: "09:00")
        records.task("After now", in: health, due: 0, at: "11:00")

        let store = await load()

        XCTAssertEqual(titles(store, "overdue"), ["Before now"])
        XCTAssertEqual(titles(store, "today"), ["After now"])
    }

    func testTheLongestOverdueComesFirst() async {
        records.task("Yesterday", in: health, priority: .urgent, due: -1)
        records.task("This morning", in: health, priority: .urgent, due: 0, at: "08:00")
        records.task("Last week", in: health, priority: .low, due: -7)
        records.task("Yesterday at noon", in: health, priority: .urgent, due: -1, at: "12:00")

        let store = await load()

        XCTAssertEqual(titles(store, "overdue"), ["Last week", "Yesterday", "Yesterday at noon", "This morning"])
    }

    func testHigherPriorityComesFirst() async {
        records.task("Low", in: health, priority: .low, due: 0)
        records.task("Urgent", in: health, priority: .urgent, due: 0)

        let store = await load()

        XCTAssertEqual(titles(store, "today"), ["Urgent", "Low"])
    }

    func testCountsStalledTasksAndOpensTheReview() async {
        records.task("Plan long runs", in: health, status: .inProgress, idle: 10)
        records.task("Fix the bike", in: health, status: .procrastinated)
        records.task("Stretch", in: health, status: .inProgress, idle: 2)

        let store = await load()
        XCTAssertEqual(store.state.procrastinatedCount, 2)

        await store.send(.procrastinatedTapped)
        XCTAssertEqual(store.state.procrastinated?.list.rows.map(\.title), ["Plan long runs", "Fix the bike"])
    }

    func testRestartingInTheReviewUpdatesTheCount() async {
        let plan = records.task("Plan long runs", in: health, status: .inProgress, idle: 10)
        let store = await load()
        await store.send(.procrastinatedTapped)

        await store.send(.procrastinated(.presented(.list(.rowAction(plan.id, .status(.inProgress))))))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.procrastinatedCount, 0)
        XCTAssertEqual(store.state.procrastinated?.list.isEmpty, true)
    }
}
