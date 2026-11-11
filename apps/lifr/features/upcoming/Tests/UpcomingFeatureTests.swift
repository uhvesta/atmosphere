import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskList
import LifrUpcoming
import XCTest

@MainActor
final class UpcomingFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")

    private func load() async -> TestStoreOf<UpcomingFeature> {
        let store = TestStore(initialState: UpcomingFeature.State()) {
            UpcomingFeature()
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

    func testGroupsTheNextTwoWeeksByDay() async {
        records.task("Buy running shoes", in: health, due: 1)
        records.task("Renew passport", in: health, due: 6)
        records.task("Call the plumber", in: health, due: 6)

        let store = await load()

        XCTAssertEqual(store.state.list.sections.map(\.title), ["Tomorrow", "Monday, Nov 9"])
        XCTAssertEqual(store.state.list.sections.map(\.rows.count), [1, 2])
    }

    func testOnADaylightSavingChangeTomorrowIsTheNextDayAndNoDayRepeats() throws {
        // 2026-11-01 00:30 in New York, the night clocks go back an hour.
        let night = Date(timeIntervalSince1970: 1_793_507_400)
        let newYork = try XCTUnwrap(TimeZone(identifier: "America/New_York"))

        let days = TaskQuery.upcoming.sections(records.workspace, night, newYork).map(\.id)

        XCTAssertEqual(days.first, "2026-11-02")
        XCTAssertEqual(days.last, "2026-11-15")
        XCTAssertEqual(Set(days).count, 14)
    }

    func testLeavesOutTodayOverdueClosedAndLaterTasks() async {
        records.task("Today", in: health, due: 0)
        records.task("Overdue", in: health, due: -1)
        records.task("Done", in: health, status: .done, due: 2)
        records.task("In three weeks", in: health, due: 21)
        records.task("Last day", in: health, due: 14)

        let store = await load()

        XCTAssertEqual(store.state.list.rows.map(\.title), ["Last day"])
    }

    func testFinishingATaskTakesItOff() async {
        let shoes = records.task("Buy running shoes", in: health, due: 1)
        let store = await load()

        await store.send(.list(.rowAction(shoes.id, .status(.done))))
        await store.receive(\.list.delegate.saved)

        XCTAssertTrue(store.state.list.isEmpty)
    }
}
