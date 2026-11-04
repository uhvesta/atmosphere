import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrToday
import XCTest

@MainActor
final class TodayFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")

    private func makeStore() -> TestStoreOf<TodayFeature> {
        let store = TestStore(initialState: TodayFeature.State()) {
            TodayFeature()
        } withDependencies: { [now, records] in
            $0.store = .inMemory(records.workspace)
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return store
    }

    private func load() async -> TestStoreOf<TodayFeature> {
        let store = makeStore()
        await store.send(.task)
        await store.receive(\.loaded)
        return store
    }

    func testShowsOverdueAndDueTodayOpenTasksOnly() async {
        records.task("Stretch", in: health, due: 0)
        records.task("Book physio", in: health, due: -2)
        records.task("Renew passport", in: health, due: 3)
        records.task("Someday", in: health)
        records.task("Already done", in: health, status: .done, due: -1)

        let store = await load()

        XCTAssertEqual(store.state.overdue.map(\.title), ["Book physio"])
        XCTAssertEqual(store.state.dueToday.map(\.title), ["Stretch"])
    }

    func testDueTimeEarlierTodayIsOverdue() async {
        records.task("Before now", in: health, due: 0, at: "09:00")
        records.task("After now", in: health, due: 0, at: "11:00")

        let store = await load()

        XCTAssertEqual(store.state.overdue.map(\.title), ["Before now"])
        XCTAssertEqual(store.state.dueToday.map(\.title), ["After now"])
    }

    func testHigherPriorityComesFirst() async {
        records.task("Low", in: health, priority: .low, due: 0)
        records.task("Urgent", in: health, priority: .urgent, due: 0)

        let store = await load()

        XCTAssertEqual(store.state.dueToday.map(\.title), ["Urgent", "Low"])
    }

    func testTogglingMarksDoneAndDropsTheTask() async {
        let stretch = records.task("Stretch", in: health, due: 0)
        let store = await load()

        await store.send(.toggled(stretch.id))
        await store.receive(\.saved)

        XCTAssertEqual(store.state.dueToday, [])
        XCTAssertEqual(store.state.workspace?.tasks[stretch.id]?.status, .done)
    }

    func testTogglingWithOpenSubtasksExplainsAndKeepsTheTask() async {
        let review = records.task("Review", in: health, due: 0)
        records.task("Read", in: health, parent: review)
        let store = await load()

        await store.send(.toggled(review.id))
        await store.receive(\.saved)

        XCTAssertNotNil(store.state.alert)
        XCTAssertEqual(store.state.dueToday.map(\.title), ["Review"])
        XCTAssertEqual(store.state.dueToday.first?.status, .todo)
    }

    func testTappingARowOpensItsDetail() async {
        let stretch = records.task("Stretch", in: health, due: 0)
        let store = await load()

        await store.send(.rowTapped(stretch.id))

        XCTAssertEqual(store.state.detail?.id, stretch.id)
    }

    func testChangesInTheDetailRefreshToday() async {
        let stretch = records.task("Stretch", in: health, due: 0)
        let store = await load()
        await store.send(.rowTapped(stretch.id))

        await store.send(.detail(.presented(.statusPicked(.done))))
        await store.receive(\.detail.delegate.saved)

        XCTAssertEqual(store.state.dueToday, [])
    }
}
