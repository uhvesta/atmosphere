import ComposableArchitecture
import Foundation
import LifrCore
import LifrQuickAdd
import LifrRemindersClient
import LifrStoreClient
import XCTest

@MainActor
final class QuickAddFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")
    private lazy var home = records.avenue("HOME", "Home")

    private func quickAdd() throws -> TestStoreOf<QuickAddFeature> {
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "QuickAddFeatureTests-\(UUID())"))
        let store = withDependencies {
            $0.defaultAppStorage = defaults
        } operation: {
            // Inside, so the remembered Avenue is read from `defaults`.
            let initial = QuickAddFeature.State(
                workspace: records.workspace, now: now, timeZone: .gmt, locale: Locale(identifier: "en_US"))
            return TestStore(initialState: initial) {
                QuickAddFeature()
            } withDependencies: { [now, records] in
                $0.store = .inMemory(records.workspace)
                $0.date.now = now
                $0.timeZone = .gmt
                $0.uuid = .incrementing
                $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
            }
        }
        store.exhaustivity = .off
        return store
    }

    func testAddsATaskDueTomorrowAndRemembersTheAvenue() async throws {
        _ = (health, home)
        let store = try quickAdd()

        await store.send(.binding(.set(\.avenueID, home.id)))
        await store.send(.binding(.set(\.title, " Call the plumber ")))
        await store.send(.binding(.set(\.due, .tomorrow)))
        await store.send(.binding(.set(\.priority, .high)))
        await store.send(.addTapped)
        let workspace = try await saved(store)

        let task = try XCTUnwrap(workspace.tasks.values.first)
        XCTAssertEqual(task.title, "Call the plumber")
        XCTAssertEqual(task.avenueID, home.id)
        XCTAssertEqual(task.priority, .high)
        XCTAssertEqual(task.dueDate?.description, "2026-11-04")
        XCTAssertEqual(store.state.lastAvenue, home.id.uuidString)
    }

    func testAddTappedTwiceAddsOneTask() async throws {
        _ = home
        let store = try quickAdd()

        await store.send(.binding(.set(\.avenueID, home.id)))
        await store.send(.binding(.set(\.title, "Call the plumber")))
        // The first save waits until the second tap is in.
        let (gate, open) = AsyncStream<Void>.makeStream()
        let saves = LockIsolated(0)
        let save = store.dependencies.store.save
        store.dependencies.store.save = { operation in
            saves.withValue { $0 += 1 }
            for await _ in gate {}
            return try await save(operation)
        }
        await store.send(.addTapped)
        await store.send(.addTapped)
        open.finish()
        await store.finish()

        XCTAssertEqual(saves.value, 1)
    }

    func testTheTitleSetsTheDueDayTimeAndPriorityUnlessAPartIsKeptAsText() async throws {
        _ = (health, home)
        let store = try quickAdd()
        await store.send(.binding(.set(\.title, "Call dentist tomorrow 3pm !high")))
        XCTAssertEqual(store.state.finalTitle, "Call dentist")
        await store.send(.partIgnored("3pm"))
        XCTAssertEqual(store.state.finalTitle, "Call dentist 3pm")
        await store.send(.addTapped)
        let workspace = try await saved(store)

        let task = try XCTUnwrap(workspace.tasks.values.first)
        XCTAssertEqual(task.title, "Call dentist 3pm")
        XCTAssertEqual(task.dueDate?.description, "2026-11-04")
        XCTAssertNil(task.dueTime)
        XCTAssertEqual(task.priority, .high)
    }

    func testANewTaskWithADueDateGetsTheDefaultReminder() async throws {
        _ = (health, home)
        let store = try quickAdd()
        store.state.$defaultReminder.withLock { $0 = .dueDayMorning }
        await store.send(.binding(.set(\.title, "Call dentist tomorrow 3pm")))
        await store.send(.addTapped)
        let workspace = try await saved(store)
        XCTAssertEqual(workspace.reminders.values.map(\.trigger), [.beforeDue(seconds: 6 * 3600)])
    }

    func testAddATimeGivesTheDueDayATime() async throws {
        _ = (health, home)
        let store = try quickAdd()
        await store.send(.binding(.set(\.title, "Call plumber")))
        await store.send(.binding(.set(\.due, .tomorrow)))
        await store.send(.binding(.set(\.addsTime, true)))
        await store.send(.binding(.set(\.pickedTime, now + 5 * 3600)))
        await store.send(.addTapped)
        let workspace = try await saved(store)
        XCTAssertEqual(workspace.tasks.values.first { $0.title == "Call plumber" }?.dueTime, try TimeOfDay("15:00"))
    }

    func testAddATimeWithNoDueDayAddsNoTime() async throws {
        _ = (health, home)
        let store = try quickAdd()
        await store.send(.binding(.set(\.title, "Call dentist")))
        await store.send(.binding(.set(\.addsTime, true)))
        await store.send(.addTapped)
        let workspace = try await saved(store)
        XCTAssertNil(workspace.tasks.values.first { $0.title == "Call dentist" }?.dueTime)
    }

    func testANewTaskWithNoDueDateGetsNoReminder() async throws {
        _ = (health, home)
        let store = try quickAdd()
        store.state.$defaultReminder.withLock { $0 = .atDue }
        await store.send(.binding(.set(\.title, "Someday")))
        await store.send(.addTapped)
        let workspace = try await saved(store)
        XCTAssertEqual(workspace.reminders.count, 0)
    }

    func testClearingTheTitleReadsTheNextOneAfresh() async throws {
        _ = (health, home)
        let store = try quickAdd()
        await store.send(.binding(.set(\.title, "Run tomorrow")))
        await store.send(.partIgnored("tomorrow"))
        await store.send(.binding(.set(\.title, "")))
        await store.send(.binding(.set(\.title, "Swim tomorrow")))
        XCTAssertEqual(store.state.finalTitle, "Swim")
    }

    func testDefaultsToTheLastAvenueUsed() async throws {
        _ = (health, home)
        let first = try quickAdd()
        XCTAssertEqual(first.state.avenueID, health.id)

        await first.send(.binding(.set(\.avenueID, home.id)))
        await first.send(.binding(.set(\.title, "Water the plants")))
        await first.send(.addTapped)
        _ = try await saved(first)

        let next = withDependencies {
            $0.defaultAppStorage = first.dependencies.defaultAppStorage
        } operation: {
            QuickAddFeature.State(
                workspace: records.workspace, now: now, timeZone: .gmt, locale: Locale(identifier: "en_US"))
        }
        XCTAssertEqual(next.avenueID, home.id)
    }

    func testChangingTheAvenueClearsAProjectFromAnother() async throws {
        let marathon = records.project("Half marathon", in: health)
        _ = home
        let store = try quickAdd()

        await store.send(.binding(.set(\.projectID, marathon.id)))
        await store.send(.binding(.set(\.avenueID, home.id)))

        XCTAssertNil(store.state.projectID)
    }

    func testSubtaskModeAddsTheTaskThenItsSubtasks() async throws {
        _ = health
        let store = try quickAdd()

        await store.send(.binding(.set(\.subtaskMode, true)))
        await store.send(.binding(.set(\.title, "Race day")))
        await store.send(.addTapped)
        _ = try await saved(store)
        XCTAssertEqual(store.state.parent?.name, "Race day")

        await store.send(.binding(.set(\.title, "Pin the bib")))
        await store.send(.addTapped)
        let workspace = try await saved(store)

        let parent = try XCTUnwrap(store.state.parent)
        XCTAssertEqual(workspace.subtasks(of: parent.id).map(\.title), ["Pin the bib"])
        XCTAssertEqual(store.state.addedSubtasks, ["Pin the bib"])
        XCTAssertEqual(store.state.title, "")
    }

    func testAProjectGoesInThePickedAvenue() async throws {
        _ = (health, home)
        let store = try quickAdd()

        await store.send(.binding(.set(\.kind, .project)))
        await store.send(.binding(.set(\.avenueID, home.id)))
        await store.send(.binding(.set(\.title, "Garden")))
        await store.send(.addTapped)
        let workspace = try await saved(store)

        XCTAssertEqual(workspace.projects.values.map(\.name), ["Garden"])
        XCTAssertEqual(workspace.projects.values.first?.avenueID, home.id)
        XCTAssertTrue(workspace.tasks.isEmpty)
    }

    func testABlankTitleCantBeAdded() async throws {
        _ = health
        let store = try quickAdd()

        await store.send(.binding(.set(\.title, "   ")))

        XCTAssertFalse(store.state.canAdd)
    }

    /// The records the last add saved.
    private func saved(_ store: TestStoreOf<QuickAddFeature>) async throws -> Workspace {
        await store.receive(\.added.success)
        return try await store.dependencies.store.load()
    }

    func testReturnTypedInTheTitleAddsTheTask() async throws {
        _ = home
        let store = try quickAdd()
        await store.send(.binding(.set(\.avenueID, home.id)))

        await store.send(.titleTyped("Buy paint tomorrow"))
        await store.send(.titleTyped("Buy paint tomorrow\n"))
        await store.receive(\.addTapped)
        let workspace = try await saved(store)

        XCTAssertEqual(workspace.tasks.values.map(\.title), ["Buy paint"])
        XCTAssertEqual(workspace.tasks.values.first?.dueDate?.description, "2026-11-04")
    }

    func testALineBreakPastedIntoTheTitleBecomesASpace() async throws {
        let store = try quickAdd()

        await store.send(.titleTyped("Buy\npaint\r\nand brushes"))

        XCTAssertEqual(store.state.title, "Buy paint and brushes")
        await store.finish()
    }

    func testPastedLinesAddATaskEachAsOneChangeReadingEachLikeTheTitle() async throws {
        _ = (health, home)
        let store = try quickAdd()

        await store.send(.binding(.set(\.avenueID, home.id)))
        await store.send(.linesPasted("- Call the plumber tomorrow\n\n2. Buy paint !high\n[ ] Fix the gate\n"))
        let workspace = try await saved(store)

        let tasks = workspace.tasks.values.sorted { $0.title < $1.title }
        XCTAssertEqual(tasks.map(\.title), ["Buy paint", "Call the plumber", "Fix the gate"])
        XCTAssertEqual(Set(tasks.map(\.avenueID)), [home.id])
        XCTAssertEqual(tasks[1].dueDate?.description, "2026-11-04")
        XCTAssertEqual(tasks[0].priority, .high)
        XCTAssertEqual(workspace.journal.count, records.workspace.journal.count + 1, "one change to undo")
    }

    func testPastingNoLinesAddsNothingAndSaysWhy() async throws {
        _ = home
        let store = try quickAdd()

        await store.send(.linesPasted(" \n- \n"))

        XCTAssertNotNil(store.state.error)
        let workspace = try await store.dependencies.store.load()
        XCTAssertTrue(workspace.tasks.isEmpty)
    }

    func testCancellingClosesWithoutAddingTheTask() async throws {
        _ = home
        let store = try quickAdd()
        let dismissed = LockIsolated(false)
        store.dependencies.dismiss = DismissEffect { dismissed.setValue(true) }

        await store.send(.binding(.set(\.title, "Call the plumber")))
        await store.send(.cancelTapped)

        XCTAssertTrue(dismissed.value)
        let workspace = try await store.dependencies.store.load()
        XCTAssertEqual(workspace.tasks.count, 0)
    }
}
