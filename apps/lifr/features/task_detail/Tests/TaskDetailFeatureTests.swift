import ComposableArchitecture
import Foundation
import LifrCore
import LifrRemindersClient
import LifrStoreClient
import LifrTaskDetail
import XCTest

@MainActor
final class TaskDetailFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")

    private func makeStore(
        _ task: LifrTask, reminders: InMemoryReminders = RemindersClient.inMemory()
    ) throws -> TestStoreOf<TaskDetailFeature> {
        let client = StoreClient.inMemory(records.workspace)
        let state = try XCTUnwrap(
            TaskDetailFeature.State(id: task.id, workspace: records.workspace, now: now, timeZone: .gmt))
        let store = TestStore(initialState: state) {
            TaskDetailFeature()
        } withDependencies: { [now] in
            // A minute passes between changes, as it would for a person.
            let clock = LockIsolated(now)
            $0.store = client
            $0.reminders = reminders.client
            $0.date = DateGenerator {
                clock.withValue {
                    $0 += 60
                    return $0
                }
            }
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
            $0.timeZone = .gmt
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return store
    }

    func testPickingAStatusSavesItAndTellsTheList() async throws {
        let stretch = records.task("Stretch", in: health)
        let store = try makeStore(stretch)

        await store.send(.statusPicked(.inProgress))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.row.status, .inProgress)
    }

    func testWontDoWithOpenSubtasksAsksThenClosesThemToo() async throws {
        let review = records.task("Review", in: health)
        records.task("Read", in: health, parent: review)
        let store = try makeStore(review)

        await store.send(.statusPicked(.wontDo))
        await store.receive(\.saved)
        XCTAssertNotNil(store.state.prompt)
        XCTAssertEqual(store.state.row.status, .todo)

        await store.send(.prompt(.presented(.dropWithSubtasks(task: review.id))))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.row.status, .wontDo)
        XCTAssertEqual(store.state.subtasks.map(\.status), [.wontDo])
    }

    func testDoneWithOpenSubtasksAsksThenFinishesDroppingThem() async throws {
        let review = records.task("Review", in: health)
        records.task("Read", in: health, parent: review)
        records.task("Reply", in: health, parent: review)
        let store = try makeStore(review)

        await store.send(.statusPicked(.done))
        await store.receive(\.saved)
        XCTAssertNotNil(store.state.prompt)
        XCTAssertEqual(store.state.row.status, .todo)

        await store.send(.prompt(.presented(.finishDroppingSubtasks(task: review.id))))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.row.status, .done)
        XCTAssertEqual(store.state.subtasks.map(\.status), [.wontDo, .wontDo])
    }

    func testReopeningASubtaskReopensTheParentWithUndo() async throws {
        let review = records.task("Review", in: health)
        let read = records.task("Read", in: health, parent: review, status: .done)
        records.setStatus(.done, of: review)
        let store = try makeStore(review)

        await store.send(.subtaskToggled(read.id))
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.row.status, .inProgress)
        XCTAssertEqual(store.state.subtasks.map(\.status), [.todo])
        XCTAssertNotNil(store.state.banner?.undo)

        await store.send(.bannerUndoTapped)
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.row.status, .done)
        XCTAssertEqual(store.state.subtasks.map(\.status), [.done])
        XCTAssertNil(store.state.banner)
    }

    func testAddingASubtaskToADoneTaskReopensItUntilUndone() async throws {
        let shoes = records.task("Buy running shoes", in: health, status: .done)
        let store = try makeStore(shoes)

        await store.send(.binding(.set(\.newSubtaskTitle, "Buy socks")))
        await store.send(.addSubtaskSubmitted)
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.subtasks.map(\.title), ["Buy socks"])
        XCTAssertEqual(store.state.row.status, .inProgress)
        XCTAssertEqual(store.state.newSubtaskTitle, "")

        await store.send(.bannerUndoTapped)
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.subtasks, [])
        XCTAssertEqual(store.state.row.status, .done)
    }

    func testABlankSubtaskTitleAddsNothing() async throws {
        let shoes = records.task("Buy running shoes", in: health)
        let store = try makeStore(shoes)

        await store.send(.binding(.set(\.newSubtaskTitle, "   ")))
        await store.send(.addSubtaskSubmitted)

        XCTAssertEqual(store.state.subtasks, [])
    }

    func testACommentShowsInTheTimelineAndTheCommentsFilter() async throws {
        let shoes = records.task("Buy running shoes", in: health)
        let store = try makeStore(shoes)

        await store.send(.binding(.set(\.newComment, "Trail **and** road")))
        await store.send(.commentSubmitted)
        await store.receive(\.delegate.saved)
        await store.send(.statusPicked(.inProgress))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(
            store.state.timeline.map(\.text), ["To Do -> In Progress", "Trail **and** road", "Created"])
        XCTAssertEqual(store.state.timeline.first?.author.hasPrefix("You on"), true)
        await store.send(.binding(.set(\.showsCommentsOnly, true)))
        XCTAssertEqual(store.state.visibleTimeline.map(\.text), ["Trail **and** road"])
    }

    func testPickingAPriorityRecordsIt() async throws {
        let shoes = records.task("Buy running shoes", in: health)
        let store = try makeStore(shoes)

        await store.send(.priorityPicked(.urgent))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.row.priority, .urgent)
    }

    func testAddsAReminderBeforeTheDueDate() async throws {
        let run = records.task("Long run", in: health, due: 2, at: "07:00")
        let store = try makeStore(run)
        XCTAssertEqual(store.state.reminderChoices, ReminderChoice.allCases)

        await store.send(.reminderPicked(.dayBeforeDue))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.reminders.map(\.fireAt), [ISO8601DateFormatter().date(from: "2026-11-04T07:00:00Z")])
        XCTAssertEqual(store.state.reminders.map(\.beforeDue), ["1 day before due"])
        XCTAssertNil(store.state.alert)
        await store.finish()
    }

    func testOffersOnlySetTimesWithoutADueDate() throws {
        let stretch = records.task("Stretch", in: health)
        let store = try makeStore(stretch)
        XCTAssertEqual(store.state.reminderChoices, [.inOneHour, .tomorrowMorning])
    }

    func testSavesTheReminderButSaysWhenNotificationsAreOff() async throws {
        let stretch = records.task("Stretch", in: health)
        let store = try makeStore(stretch, reminders: RemindersClient.inMemory(permission: .denied))

        await store.send(.reminderPicked(.tomorrowMorning))
        await store.receive(\.delegate.saved)
        await store.receive(\.notificationsOff)

        XCTAssertEqual(store.state.reminders.count, 1)
        XCTAssertNotNil(store.state.alert)
    }

    func testRemovesAReminder() async throws {
        let stretch = records.task("Stretch", in: health)
        records.reminder(.at(now + 3600), on: stretch)
        let store = try makeStore(stretch)
        XCTAssertEqual(store.state.reminders.count, 1)

        await store.send(.reminderRemoved(store.state.reminders[0].id))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.reminders, [])
    }

    func testRenamingSavesTheNewTitle() async throws {
        let stretch = records.task("Stretch", in: health)
        let store = try makeStore(stretch)

        await store.send(.binding(.set(\.title, "  Stretch for 10 minutes ")))
        await store.send(.titleSubmitted)
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.row.title, "Stretch for 10 minutes")
    }

    func testUndoInTheActivityPutsBackAChangeThatIsStillCurrent() async throws {
        let stretch = records.task("Stretch", in: health)
        let store = try makeStore(stretch)
        await store.send(.binding(.set(\.title, "Stretch for 10 minutes")))
        await store.send(.titleSubmitted)
        await store.receive(\.delegate.saved)
        let rename = try XCTUnwrap(store.state.timeline.first { $0.canUndo })

        await store.send(.historyUndoTapped(rename.id))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.row.title, "Stretch")
        // The title isn't the rename's any more, so it no longer offers Undo.
        XCTAssertFalse(store.state.timeline.contains { $0.id == rename.id && $0.canUndo })
    }

    func testAnEmptyTitleIsPutBack() async throws {
        let stretch = records.task("Stretch", in: health)
        let store = try makeStore(stretch)

        await store.send(.binding(.set(\.title, "   ")))
        await store.send(.titleSubmitted)

        XCTAssertEqual(store.state.title, "Stretch")
    }

    func testEditingTheDescriptionSavesOnlyWhenSaved() async throws {
        let stretch = records.task("Stretch", in: health)
        let store = try makeStore(stretch)

        await store.send(.editDetailsTapped)
        await store.send(.binding(.set(\.detailsDraft, "Hips, then *back*.")))
        await store.send(.detailsCancelled)
        XCTAssertEqual(store.state.details, "")

        await store.send(.editDetailsTapped)
        await store.send(.binding(.set(\.detailsDraft, "Hips, then *back*.")))
        await store.send(.detailsSaved)
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.details, "Hips, then *back*.")
        XCTAssertNil(store.state.detailsDraft)
    }

    func testRemovingTheDueDateRemovesItsTime() async throws {
        let stretch = records.task("Stretch", in: health, due: 1, at: "07:00")
        let store = try makeStore(stretch)
        let friday = try CalendarDate(year: 2026, month: 11, day: 6)

        await store.send(.dueDatePicked(friday))
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.dueDate, friday)
        XCTAssertEqual(store.state.dueTime, try TimeOfDay("07:00"))

        await store.send(.dueDatePicked(nil))
        await store.receive(\.delegate.saved)
        XCTAssertNil(store.state.dueDate)
        XCTAssertNil(store.state.dueTime)
    }

    func testMovingToAnotherAvenueTakesTheSubtasksAndLeavesTheProject() async throws {
        let fitness = records.project("Fitness", in: health)
        let home = records.avenue("HOME", "Home")
        let stretch = records.task("Stretch", in: health, project: fitness)
        records.task("Hips", in: health, parent: stretch)
        let store = try makeStore(stretch)
        XCTAssertEqual(store.state.projectChoices.map(\.name), ["Fitness"])

        await store.send(.avenuePicked(home.id))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.avenueID, home.id)
        XCTAssertNil(store.state.projectID)
        XCTAssertEqual(store.state.row.avenue, "Home")
        XCTAssertEqual(store.state.subtasks.map(\.avenue), ["Home"])
    }

    func testPickingAProjectInAnotherAvenueMovesTheTaskAndSubtasksThereInOneChange() async throws {
        let home = records.avenue("HOME", "Home")
        let garden = records.project("Garden", in: home)
        let stretch = records.task("Stretch", in: health)
        let hips = records.task("Hips", in: health, parent: stretch)
        let store = try makeStore(stretch)
        XCTAssertEqual(store.state.otherProjectChoices.map(\.avenue.name), ["Home"])

        await store.send(.projectPicked(garden.id))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.avenueID, home.id)
        XCTAssertEqual(store.state.projectID, garden.id)
        let workspace = try await store.dependencies.store.load()
        XCTAssertEqual(workspace.tasks[hips.id]?.avenueID, home.id)
        XCTAssertEqual(workspace.tasks[hips.id]?.projectID, garden.id)
        XCTAssertEqual(workspace.journal.count, records.workspace.journal.count + 1, "the move is one change")
    }

    func testLabelsAreToggledAndNewOnesReuseAnExistingName() async throws {
        let errands = records.label("errands")
        let stretch = records.task("Stretch", in: health)
        let store = try makeStore(stretch)

        await store.send(.labelToggled(errands.id))
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.labels, ["errands"])

        await store.send(.binding(.set(\.newLabelName, "Errands")))
        await store.send(.newLabelSubmitted)
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.labelChoices.count, 1)

        await store.send(.binding(.set(\.newLabelName, "quick")))
        await store.send(.newLabelSubmitted)
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.labels, ["errands", "quick"])

        await store.send(.labelToggled(errands.id))
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.labels, ["quick"])
    }

    func testASubtaskOpensInItsOwnDetailAndItsChangesShowInTheParent() async throws {
        let stretch = records.task("Stretch", in: health)
        let hips = records.task("Hips", in: health, parent: stretch)
        let store = try makeStore(stretch)

        await store.send(.subtaskTapped(hips.id))
        await store.receive(\.subtaskLoaded)
        XCTAssertEqual(store.state.subtask?.row.title, "Hips")

        await store.send(.subtask(.presented(.statusPicked(.inProgress))))
        await store.receive(\.subtask.delegate.saved)
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.subtasks.map(\.status), [.inProgress])
    }
}
