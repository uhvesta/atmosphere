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

    func testSnoozeAddsOneReminderAtTheTimeEvenWhileLocked() async throws {
        let (shortcuts, store) = shortcuts(locked: true)
        let loaded = try await store.load()
        let run = try XCTUnwrap(loaded.tasks.values.first { $0.title == "Long run" })

        try await shortcuts.snooze(run.id, until: now + 3600)

        let reminders = try await store.load().reminders.values
        XCTAssertEqual(reminders.map(\.trigger), [.at(now + 3600)])
        XCTAssertEqual(reminders.map(\.taskID), [run.id])
    }

    func testThePlanPromptNamesTheGoalAndAvenuesButNoTask() async throws {
        let (shortcuts, _) = shortcuts()

        let prompt = try await shortcuts.planPrompt(goal: "Run a half marathon")

        XCTAssertTrue(prompt.contains("Run a half marathon"))
        XCTAssertTrue(prompt.contains("HEALTH"))
        XCTAssertFalse(prompt.contains("Long run"))
    }

    func testThePlanPromptIsRefusedWhileLocked() async throws {
        let (shortcuts, _) = shortcuts(locked: true)
        do {
            _ = try await shortcuts.planPrompt(goal: "Run a half marathon")
            XCTFail("Making a plan prompt while locked should be refused")
        } catch let error as ShortcutError {
            XCTAssertEqual(error, .locked)
        }
    }

    func testDoneIsRefusedWhileLocked() async throws {
        let (shortcuts, store) = shortcuts(locked: true)
        let loaded = try await store.load()
        let run = try XCTUnwrap(loaded.tasks.values.first { $0.title == "Long run" })
        do {
            _ = try await shortcuts.finish(run.id)
            XCTFail("Finishing while locked should be refused")
        } catch let error as ShortcutError {
            XCTAssertEqual(error, .locked)
        }
        let after = try await store.load()
        XCTAssertEqual(after.tasks[run.id]?.status, .todo)
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

    func testChangesAStatusAndMovesTheDueDate() async throws {
        let (shortcuts, store) = shortcuts()
        let runs = try await shortcuts.openTasks(matching: "run")
        let run = try XCTUnwrap(runs.first)

        _ = try await shortcuts.setStatus(.inProgress, of: run.id)
        _ = try await shortcuts.reschedule(run.id, to: now + 3 * 24 * 60 * 60)

        let task = try await store.load().tasks[run.id]
        XCTAssertEqual(task?.status, .inProgress)
        XCTAssertEqual(task?.dueDate, CalendarDate(now, in: .gmt).adding(days: 3))
        _ = try await shortcuts.reschedule(run.id, to: nil)
        let undated = try await store.load().tasks[run.id]
        XCTAssertNil(undated?.dueDate)
    }

    func testCommentsOnATaskButNotWithNothingToSay() async throws {
        let (shortcuts, store) = shortcuts()
        let runs = try await shortcuts.openTasks(matching: "run")
        let run = try XCTUnwrap(runs.first)

        _ = try await shortcuts.comment(" Felt good ", on: run.id)
        do {
            _ = try await shortcuts.comment("  ", on: run.id)
            XCTFail("Expected it to fail")
        } catch {
            XCTAssertEqual(error as? ShortcutError, .noComment)
        }

        let comments = try await store.load().comments.values.filter { $0.taskID == run.id }
        XCTAssertEqual(comments.map(\.body), ["Felt good"])
    }

    func testATaskInTheTrashCantBeChanged() async throws {
        let (shortcuts, store) = shortcuts()
        let runs = try await shortcuts.openTasks(matching: "run")
        let run = try XCTUnwrap(runs.first)
        _ = try await store.moveToTrash(run.id)

        do {
            _ = try await shortcuts.setStatus(.done, of: run.id)
            XCTFail("Expected it to fail")
        } catch {
            XCTAssertEqual(error as? ShortcutError, .noTask)
        }
        let status = try await store.load().tasks[run.id]?.status
        XCTAssertEqual(status, .todo)
    }

    func testWithTheLockOnNoTaskIsChanged() async throws {
        let (unlocked, store) = shortcuts()
        let runs = try await unlocked.openTasks(matching: "run")
        let run = try XCTUnwrap(runs.first)
        let locked = Shortcuts(store: store, locksApp: { true }, now: { Date() }, timeZone: .gmt)

        for change in [
            { _ = try await locked.setStatus(.wontDo, of: run.id) },
            { _ = try await locked.reschedule(run.id, to: nil) },
            { _ = try await locked.comment("Hi", on: run.id) },
        ] as [() async throws -> Void] {
            do {
                try await change()
                XCTFail("Expected it to fail")
            } catch {
                XCTAssertEqual(error as? ShortcutError, .locked)
            }
        }
        let task = try await store.load().tasks[run.id]
        XCTAssertEqual(task?.status, .todo)
        XCTAssertNotNil(task?.dueDate)
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

    /// HEALTH has Long run (today, in Half marathon, labelled Outdoors),
    /// Stretch (two days ago), Book physio (in five days) and Old shoes
    /// (Done); WORK has Write report (tomorrow, Outdoors) and Old memo in
    /// the Trash.
    private func findable(locked: Bool = false) -> Shortcuts {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let health = records.avenue("HEALTH", "Health")
        let work = records.avenue("WORK", "Work")
        let marathon = records.project("Half marathon", in: health)
        let outdoors = records.label("Outdoors")
        records.task("Long run", in: health, project: marathon, due: 0, labels: [outdoors])
        records.task("Stretch", in: health, due: -2)
        records.task("Book physio", in: health, due: 5)
        records.task("Old shoes", in: health, status: .done)
        records.task("Write report", in: work, due: 1, labels: [outdoors])
        records.trash(records.task("Old memo", in: work, due: 0))
        let store = StoreClient.inMemory(records.workspace)
        let now = now
        return Shortcuts(store: store, locksApp: { locked }, now: { now }, timeZone: .gmt)
    }

    func testFindsOpenTasksInDateOrderLeavingOutTheTrash() async throws {
        let found = try await findable().find(.init())

        XCTAssertEqual(found.map(\.name), ["Stretch", "Long run", "Write report", "Book physio"])
    }

    func testFindsByStatusAvenueProjectAndLabel() async throws {
        let shortcuts = findable()
        let avenues = try await shortcuts.avenues()
        let work = try XCTUnwrap(avenues.first { $0.name == "Work" })
        let projects = try await shortcuts.projects()
        let labels = try await shortcuts.labels()
        let project = try XCTUnwrap(projects.first)
        let label = try XCTUnwrap(labels.first)

        let done = try await shortcuts.find(.init(status: .done))
        let inWork = try await shortcuts.find(.init(avenue: work.id))
        let inProject = try await shortcuts.find(.init(project: project.id))
        let labelled = try await shortcuts.find(.init(label: label.id))

        XCTAssertEqual(done.map(\.name), ["Old shoes"])
        XCTAssertEqual(inWork.map(\.name), ["Write report"])
        XCTAssertEqual(inProject.map(\.name), ["Long run"])
        XCTAssertEqual(labelled.map(\.name), ["Long run", "Write report"])
        XCTAssertEqual(project.detail, "Health")
    }

    func testFindsTasksDueByADayLeavingOutUndatedOnes() async throws {
        let found = try await findable().find(.init(dueBy: now + 24 * 60 * 60))

        XCTAssertEqual(found.map(\.name), ["Stretch", "Long run", "Write report"])
    }

    func testGivesATasksDetails() async throws {
        let shortcuts = findable()
        let open = try await shortcuts.find(.init())
        let run = try XCTUnwrap(open.first { $0.name == "Long run" })

        let details = try await shortcuts.details(of: run.id)

        XCTAssertEqual(details.status, .todo)
        XCTAssertEqual(details.due, CalendarDate(now, in: .gmt))
        XCTAssertEqual(details.avenue, "Health")
        XCTAssertEqual(details.project, "Half marathon")
        XCTAssertEqual(details.labels, ["Outdoors"])
        XCTAssertTrue(details.text.contains("Long run"))
        XCTAssertTrue(details.text.contains(details.code))
    }

    func testFindingAndDetailsAreRefusedWhileLocked() async throws {
        let found = try await findable().find(.init())
        let open = try XCTUnwrap(found.first)
        let shortcuts = findable(locked: true)
        do {
            _ = try await shortcuts.find(.init())
            XCTFail("Finding tasks while locked should be refused")
        } catch let error as ShortcutError {
            XCTAssertEqual(error, .locked)
        }
        do {
            _ = try await shortcuts.details(of: open.id)
            XCTFail("Reading a task while locked should be refused")
        } catch let error as ShortcutError {
            XCTAssertEqual(error, .locked)
        }
    }

    func testAMissingTaskHasNoDetails() async throws {
        do {
            _ = try await findable().details(of: UUID())
            XCTFail("A missing task should have no details")
        } catch let error as ShortcutError {
            XCTAssertEqual(error, .noTask)
        }
    }
}
