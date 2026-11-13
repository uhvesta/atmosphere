import ComposableArchitecture
import Foundation
import LifrCalendar
import LifrCore
import LifrStoreClient
import LifrTaskList
import XCTest

@MainActor
final class CalendarFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")

    private func day(_ string: String) throws -> CalendarDate { try CalendarDate(string) }

    private func load() async -> TestStoreOf<CalendarFeature> {
        let store = TestStore(initialState: CalendarFeature.State()) {
            CalendarFeature()
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

    private func dueCount(_ store: TestStoreOf<CalendarFeature>, on date: CalendarDate) -> Int? {
        store.state.weeks.joined().first { $0.date == date }?.colors.count
    }

    func testOpensOnTodayWithTodaysTasksUnderTheMonth() async throws {
        records.task("Run", in: health, due: 0)
        records.task("Stretch", in: health, due: 1)

        let store = await load()

        XCTAssertEqual(store.state.selected, try day("2026-11-03"))
        XCTAssertEqual(store.state.month, try day("2026-11-01"))
        XCTAssertEqual(store.state.list.rows.map(\.title), ["Run"])
        XCTAssertEqual(dueCount(store, on: try day("2026-11-04")), 1)
    }

    func testTheWeeksFollowThePersonsFirstWeekday() async throws {
        let store = await load()
        XCTAssertEqual(store.state.weeks.first?.first?.date, try day("2026-11-01"))

        await store.send(.firstWeekdayChanged(2))

        XCTAssertEqual(store.state.weeks.first?.first?.date, try day("2026-10-26"))
    }

    func testWeeksStartOnTheFirstWeekdayAsked() throws {
        let november = try day("2026-11-01")

        let sundayFirst = records.workspace.calendarWeeks(month: november, firstWeekday: 1, now: now, timeZone: .gmt)
        let mondayFirst = records.workspace.calendarWeeks(month: november, firstWeekday: 2, now: now, timeZone: .gmt)

        // November 2026 starts on a Sunday and has 30 days.
        XCTAssertEqual(sundayFirst.first?.first?.date, november)
        XCTAssertEqual(sundayFirst.count, 5)
        XCTAssertEqual(mondayFirst.first?.first?.date, try day("2026-10-26"))
        XCTAssertEqual(mondayFirst.count, 6)
        XCTAssertTrue(mondayFirst.allSatisfy { $0.count == 7 })
    }

    func testATaskDueOnTheLastDayIsInThatMonthAndNotTheNext() throws {
        records.task("Pay rent", in: health, due: 27)
        let workspace = records.workspace

        let november = workspace.calendarWeeks(month: try day("2026-11-01"), firstWeekday: 1, now: now, timeZone: .gmt)
        let december = workspace.calendarWeeks(month: try day("2026-12-01"), firstWeekday: 1, now: now, timeZone: .gmt)

        XCTAssertEqual(november.joined().first { $0.date == (try? day("2026-11-30")) }?.colors.count, 1)
        XCTAssertTrue(december.joined().filter(\.isInMonth).allSatisfy(\.colors.isEmpty))
    }

    func testClosedTasksAreLeftOutAndOverdueOnesStayOnTheirDay() async throws {
        records.task("Done", in: health, status: .done, due: 0)
        records.task("Late", in: health, due: -1)

        let store = await load()

        XCTAssertEqual(dueCount(store, on: try day("2026-11-03")), 0)
        let monday = store.state.weeks.joined().first { $0.date == (try? day("2026-11-02")) }
        XCTAssertEqual(monday?.colors.count, 1)
        XCTAssertEqual(monday?.hasOverdue, true)
    }

    func testSelectingADayShowsItsTasks() async throws {
        records.task("Run", in: health, due: 0)
        records.task("Dentist", in: health, due: 7)
        let store = await load()

        await store.send(.daySelected(try day("2026-11-10")))

        XCTAssertEqual(store.state.list.rows.map(\.title), ["Dentist"])
        XCTAssertEqual(store.state.list.query.adds?.dueDate, try day("2026-11-10"))
    }

    func testAllMonthsCentreOnTheOpenOneAndPickingADayGoesBackToIt() async throws {
        records.task("Dentist", in: health, due: 40)
        let store = await load()

        await store.send(.monthsTapped)
        let months = try XCTUnwrap(store.state.months)
        XCTAssertEqual(months.count, 49)
        XCTAssertEqual(months[24].id, try day("2026-11-01"))
        XCTAssertEqual(months.first?.id, try day("2024-11-01"))
        let december = months[25].weeks.joined().first { $0.date == (try? day("2026-12-13")) }
        XCTAssertEqual(december?.colors.count, 1)

        await store.send(.daySelected(try day("2026-12-13")))
        XCTAssertNil(store.state.months)
        XCTAssertEqual(store.state.month, try day("2026-12-01"))
        XCTAssertEqual(store.state.list.rows.map(\.title), ["Dentist"])

        await store.send(.monthsTapped)
        await store.send(.monthsDismissed)
        XCTAssertNil(store.state.months)
    }

    func testPickingAMonthOnTheLongPageGoesBackToItOnTodayOrItsFirstDay() async throws {
        let store = await load()

        await store.send(.monthsTapped)
        await store.send(.monthPicked(try day("2027-02-01")))
        XCTAssertNil(store.state.months)
        XCTAssertEqual(store.state.selected, try day("2027-02-01"))

        await store.send(.monthsTapped)
        await store.send(.monthPicked(try day("2026-11-01")))
        XCTAssertEqual(store.state.selected, try day("2026-11-03"))
    }

    func testAnotherMonthSelectsItsFirstDayAndTodayComesBack() async throws {
        let store = await load()

        await store.send(.monthStepped(1))
        XCTAssertEqual(store.state.month, try day("2026-12-01"))
        XCTAssertEqual(store.state.selected, try day("2026-12-01"))

        await store.send(.monthStepped(-2))
        XCTAssertEqual(store.state.selected, try day("2026-10-01"))

        await store.send(.todayTapped)
        XCTAssertEqual(store.state.month, try day("2026-11-01"))
        XCTAssertEqual(store.state.selected, try day("2026-11-03"))
    }

    func testDroppingATaskOnAnotherDayMakesItDueThenAndKeepsItsTime() async throws {
        let run = records.task("Run", in: health, due: 0, at: "09:00")
        let store = await load()

        await store.send(.dropped(run.id, on: try day("2026-11-10")))
        await store.receive(\.list.saved)

        let task = store.state.list.workspace?.tasks[run.id]
        XCTAssertEqual(task?.dueDate, try day("2026-11-10"))
        XCTAssertEqual(task?.dueTime, try TimeOfDay("09:00"))
        XCTAssertEqual(store.state.list.rows.map(\.title), [])
        XCTAssertEqual(dueCount(store, on: try day("2026-11-10")), 1)
    }

    func testTheDragTipShowsUnderADayWithTasksUntilClosed() async {
        records.task("Run", in: health, due: 0)
        let store = await load()
        XCTAssertTrue(store.state.showsDragTip)

        await store.send(.dragTipClosed)

        XCTAssertFalse(store.state.showsDragTip)
    }

    func testDroppingATaskCountsAsSeeingTheDragTip() async throws {
        let run = records.task("Run", in: health, due: 0)
        let store = await load()

        await store.send(.dropped(run.id, on: try day("2026-11-10")))
        await store.receive(\.list.saved)

        XCTAssertFalse(store.state.showsDragTip)
    }

    func testDroppingATaskOnItsOwnDaySavesNothing() async throws {
        let run = records.task("Run", in: health, due: 0)
        let store = await load()
        let before = store.state.list.workspace

        await store.send(.dropped(run.id, on: try day("2026-11-03")))
        await store.finish()

        XCTAssertEqual(store.state.list.workspace, before)
    }

    func testATaskThatRunsOverDaysMovesAsAWhole() async throws {
        let trip = records.task("Pack for the trip", in: health, due: 2, starts: 0)
        let store = await load()

        await store.send(.dropped(trip.id, on: try day("2026-11-12")))
        await store.receive(\.list.saved)

        let task = store.state.list.workspace?.tasks[trip.id]
        XCTAssertEqual(task?.startDate, try day("2026-11-10"))
        XCTAssertEqual(task?.dueDate, try day("2026-11-12"))
    }
}
