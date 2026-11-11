import ComposableArchitecture
import Foundation
import LifrCore
import LifrShortcuts
import LifrStoreClient
import XCTest

final class ShortcutsTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    override func invokeTest() {
        withDependencies { [now] in
            $0.date.now = now
            $0.uuid = .incrementing
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
        } operation: {
            super.invokeTest()
        }
    }

    private func shortcuts(locked: Bool = false) -> (Shortcuts, StoreClient) {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let health = records.avenue("HEALTH", "Health")
        records.task("Long run", in: health, due: 0)
        records.task("Stretch", in: health, due: -2)
        records.task("Book physio", in: health, due: 5)
        let store = StoreClient.inMemory(records.workspace)
        let now = now
        return (Shortcuts(store: store, locksApp: { locked }, now: { now }, timeZone: .gmt), store)
    }

    func testAddsATaskDueToday() async throws {
        let (shortcuts, store) = shortcuts()

        let added = try await shortcuts.add("  Buy milk ", avenue: nil, due: .today)

        let task = try await store.load().tasks[added.id]
        XCTAssertEqual(task?.title, "Buy milk")
        XCTAssertEqual(task?.dueDate, CalendarDate(now, in: .gmt))
        let today = try await shortcuts.today()
        XCTAssertTrue(today.titles.contains("Buy milk"))
    }

    func testAddsATaskInAnAvenueForTomorrow() async throws {
        let (shortcuts, store) = shortcuts()
        let avenues = try await shortcuts.avenues()
        let health = try XCTUnwrap(avenues.first)

        let added = try await shortcuts.add("Ice bath", avenue: health.id, due: .tomorrow)

        let task = try await store.load().tasks[added.id]
        XCTAssertEqual(task?.avenueID, health.id)
        XCTAssertEqual(task?.dueDate, CalendarDate(now + 24 * 60 * 60, in: .gmt))
    }

    func testAnEmptyTitleAddsNothing() async throws {
        let (shortcuts, store) = shortcuts()
        let before = try await store.load().tasks.count

        do {
            _ = try await shortcuts.add("   ", avenue: nil, due: .none)
            XCTFail("Expected it to fail")
        } catch {
            XCTAssertEqual(error as? ShortcutError, .noTitle)
        }
        let after = try await store.load().tasks.count
        XCTAssertEqual(after, before)
    }

    func testWithNoAvenuePickedAddsToTheInbox() async throws {
        let store = StoreClient.inMemory(RecordsBuilder(now: now, timeZone: .gmt).workspace)
        let now = now
        let shortcuts = Shortcuts(store: store, locksApp: { false }, now: { now }, timeZone: .gmt)

        _ = try await shortcuts.add("Buy milk", avenue: nil, due: .today)
        _ = try await shortcuts.add("Buy eggs", avenue: nil, due: .none)

        let workspace = try await store.load()
        XCTAssertEqual(workspace.avenues.count, 1)
        XCTAssertEqual(Set(workspace.tasks.values.map(\.avenueID)), [Avenue.inboxID])
        XCTAssertEqual(workspace.tasks.count, 2)
    }

    func testTodayCountsAndNamesItsTasks() async throws {
        let today = try await shortcuts().0.today()

        XCTAssertEqual(today.count, 2)
        XCTAssertEqual(today.overdue, 1)
        XCTAssertEqual(Set(today.titles), ["Long run", "Stretch"])
    }

    func testWithTheLockOnTodayGivesCountsOnly() async throws {
        let today = try await shortcuts(locked: true).0.today()

        XCTAssertEqual(today.count, 2)
        XCTAssertEqual(today.overdue, 1)
        XCTAssertEqual(today.titles, [])
        let said = TodaySummary(today).text
        XCTAssertFalse(said.contains("Long run"))
        XCTAssertFalse(said.contains("Stretch"))
    }

    func testFindsOpenTasksByWordsOrSuggestsToday() async throws {
        let shortcuts = shortcuts().0

        let found = try await shortcuts.openTasks(matching: "physio")
        XCTAssertEqual(found.map(\.name), ["Book physio"])
        let suggested = try await shortcuts.openTasks(matching: nil)
        XCTAssertEqual(Set(suggested.map(\.name)), ["Long run", "Stretch"])
    }

    func testWithTheLockOnNoTaskIsListed() async throws {
        let (unlocked, _) = shortcuts()
        let runs = try await unlocked.openTasks(matching: "run")
        let id = try XCTUnwrap(runs.first?.id)
        let locked = shortcuts(locked: true).0

        let found = try await locked.openTasks(matching: "run")
        let byID = try await locked.tasks([id])
        XCTAssertEqual(found, [])
        XCTAssertEqual(byID, [])
    }

    func testFinishesATask() async throws {
        let (shortcuts, store) = shortcuts()
        let runs = try await shortcuts.openTasks(matching: "run")
        let run = try XCTUnwrap(runs.first)

        _ = try await shortcuts.finish(run.id)

        let status = try await store.load().tasks[run.id]?.status
        XCTAssertEqual(status, .done)
    }

    func testWithTheLockOnNothingIsFinished() async throws {
        let (unlocked, store) = shortcuts()
        let runs = try await unlocked.openTasks(matching: "run")
        let run = try XCTUnwrap(runs.first)
        let locked = Shortcuts(store: store, locksApp: { true }, now: { Date() }, timeZone: .gmt)

        do {
            _ = try await locked.finish(run.id)
            XCTFail("Expected it to fail")
        } catch {
            XCTAssertEqual(error as? ShortcutError, .locked)
        }
        let status = try await store.load().tasks[run.id]?.status
        XCTAssertEqual(status, .todo)
    }
}
