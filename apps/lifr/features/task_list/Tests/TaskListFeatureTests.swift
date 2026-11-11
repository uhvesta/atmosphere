import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskDetail
import LifrTaskList
import XCTest

@MainActor
final class TaskListFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")

    /// Open tasks, one section per status.
    private let open = TaskQuery(
        key: "open", empty: .init("Nothing open", systemImage: "checkmark", description: "")
    ) { workspace, now, timeZone in
        TaskStatus.allCases.filter { !$0.isClosed }.map { status in
            let tasks = workspace.listedTasks.filter {
                !$0.isSubtask && $0.isOpen && $0.effectiveStatus(now: now, settings: workspace.settings) == status
            }
            return TaskSection(
                id: status.rawValue, title: status.rawValue, rows: workspace.rows(tasks, now: now, timeZone: timeZone))
        }
    }

    private func load(
        reordering: Bool = false, client: StoreClient? = nil
    ) async -> TestStoreOf<TaskListFeature> {
        var query = open
        query.reorders = reordering
        let store = TestStore(initialState: TaskListFeature.State(query: query)) {
            TaskListFeature()
        } withDependencies: { [now, records] in
            $0.store = client ?? .inMemory(records.workspace)
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)
        return store
    }

    func testShowsOnlySectionsWithRows() async {
        records.task("Stretch", in: health)
        records.task("Plan long runs", in: health, status: .inProgress)
        records.task("Already done", in: health, status: .done)

        let store = await load()

        XCTAssertEqual(store.state.sections.map(\.id), ["todo", "inProgress"])
    }

    func testSortingByTitleOrdersEachSectionAToZ() async {
        records.task("Stretch", in: health, priority: .high)
        records.task("Buy shoes", in: health)
        records.task("Long run", in: health, priority: .urgent)
        let store = await load()
        XCTAssertEqual(store.state.rows.map(\.title), ["Long run", "Stretch", "Buy shoes"])

        await store.send(.sortPicked(.title))

        XCTAssertEqual(store.state.rows.map(\.title), ["Buy shoes", "Long run", "Stretch"])
    }

    func testAListThatCantBeReorderedOffersNoManualSortAndIgnoresDrags() async {
        records.task("One", in: health)
        records.task("Two", in: health)
        let store = await load()
        XCTAssertFalse(store.state.sorts.contains(.manual))

        await store.send(.moved(section: "todo", from: [1], to: 0))

        XCTAssertEqual(store.state.rows.map(\.title), ["One", "Two"])
    }

    func testDraggingTheThirdRowToTheTopInManualOrderKeepsThatOrderAfterReloading() async throws {
        for title in ["One", "Two", "Three"] { records.task(title, in: health) }
        let client = StoreClient.inMemory(records.workspace)
        let store = await load(reordering: true, client: client)
        await store.send(.sortPicked(.manual))

        await store.send(.moved(section: "todo", from: [2], to: 0))
        XCTAssertEqual(store.state.rows.map(\.title), ["Three", "One", "Two"])
        await store.receive(\.saved)

        let reloaded = TaskListFeature.State(
            query: store.state.query, workspace: try await client.load(), now: now, timeZone: .gmt)
        XCTAssertEqual(reloaded.rows.map(\.title), ["Three", "One", "Two"])
    }

    func testAnEmptyQueryIsEmpty() async {
        let store = await load()

        XCTAssertTrue(store.state.isEmpty)
    }

    func testSwipingRightStartsTheTaskAndTellsTheOtherLists() async {
        let stretch = records.task("Stretch", in: health)
        let store = await load()

        await store.send(.rowAction(stretch.id, .status(.inProgress)))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.sections.map(\.id), ["inProgress"])
    }

    func testTheCheckboxAsksWhichStatusInsteadOfFinishing() async {
        let stretch = records.task("Stretch", in: health)
        let store = await load()

        await store.send(.toggled(stretch.id))
        XCTAssertNotNil(store.state.statusPicker)
        XCTAssertEqual(store.state.workspace?.tasks[stretch.id]?.status, .todo)

        await store.send(.statusPicker(.presented(StatusPick(task: stretch.id, status: .done))))
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.workspace?.tasks[stretch.id]?.status, .done)
    }

    func testTheStatusSwipeOffersEveryOtherStatusAndSetsThePickedOne() async {
        let stretch = records.task("Stretch", in: health)
        let store = await load()

        await store.send(.rowAction(stretch.id, .pickStatus))
        XCTAssertNotNil(store.state.statusPicker)

        await store.send(.statusPicker(.presented(StatusPick(task: stretch.id, status: .procrastinated))))
        await store.receive(\.delegate.saved)
        XCTAssertNil(store.state.statusPicker)
        XCTAssertEqual(store.state.workspace?.tasks[stretch.id]?.status, .procrastinated)
    }

    func testDoneWithOpenSubtasksAsksAndCanShowThem() async {
        let review = records.task("Review", in: health)
        records.task("Read", in: health, parent: review)
        let store = await load()

        await store.send(.rowAction(review.id, .status(.done)))
        await store.receive(\.saved)
        XCTAssertNotNil(store.state.prompt)

        await store.send(.prompt(.presented(.showSubtasks(task: review.id))))
        await store.receive(\.rowTapped)

        XCTAssertEqual(store.state.detail?.subtasks.map(\.title), ["Read"])
    }

    func testWontDoPlusFinishClosesEverything() async {
        let review = records.task("Review", in: health)
        records.task("Read", in: health, parent: review)
        let store = await load()

        await store.send(.rowAction(review.id, .status(.done)))
        await store.receive(\.saved)
        await store.send(.prompt(.presented(.finishDroppingSubtasks(task: review.id))))
        await store.receive(\.delegate.saved)

        XCTAssertTrue(store.state.isEmpty)
        XCTAssertEqual(store.state.workspace?.tasks[review.id]?.status, .done)
    }

    func testRescheduleSetsTheNewDueDate() async throws {
        let stretch = records.task("Stretch", in: health)
        let store = await load()

        await store.send(.rowAction(stretch.id, .reschedule))
        XCTAssertEqual(store.state.reschedule?.title, "Stretch")
        let picked = now.addingTimeInterval(3 * 24 * 60 * 60)
        await store.send(.reschedule(.presented(.binding(.set(\.date, picked)))))
        await store.send(.reschedule(.presented(.saveTapped)))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.workspace?.tasks[stretch.id]?.dueDate, try CalendarDate("2026-11-06"))
        XCTAssertNil(store.state.reschedule)
    }

    func testReopeningAParentFromTheListOffersUndo() async {
        let review = records.task("Review", in: health)
        let read = records.task("Read", in: health, parent: review, status: .done)
        records.setStatus(.done, of: review)
        let store = await load()

        await store.send(.rowTapped(review.id))
        await store.send(.detail(.presented(.subtaskToggled(read.id))))
        await store.send(.detail(.presented(.statusPicker(.presented(StatusPick(task: read.id, status: .todo))))))
        await store.receive(\.detail.delegate.saved)

        XCTAssertEqual(store.state.sections.map(\.id), ["inProgress"])
    }

    func testAChangeFromElsewhereShowsInTheListAndTheOpenTask() async {
        let stretch = records.task("Stretch", in: health)
        let store = await load()
        await store.send(.rowTapped(stretch.id))

        var workspace = records.workspace
        _ = try? workspace.setStatus(
            .inProgress, of: stretch.id, in: Context(actor: .user(device: "Mac", surface: .mac), now: now))
        await store.send(.show(Snapshot(workspace, number: 1)))

        XCTAssertEqual(store.state.sections.map(\.id), ["inProgress"])
        XCTAssertEqual(store.state.detail?.row.status, .inProgress)
    }

    func testTheOpenTaskClosesOnceItsTrashedHereOrElsewhere() async {
        let stretch = records.task("Stretch", in: health)
        let store = await load()
        await store.send(.rowTapped(stretch.id))
        XCTAssertNotNil(store.state.detail)

        var workspace = records.workspace
        _ = try? workspace.moveToTrash(stretch.id, in: Context(actor: .user(device: "Mac", surface: .mac), now: now))
        await store.send(.show(Snapshot(workspace, number: 1)))

        XCTAssertNil(store.state.detail)
        XCTAssertEqual(store.state.sections, [])
    }

    func testUndoFromTheBannerRestoresTheParent() async {
        let review = records.task("Review", in: health)
        let read = records.task("Read", in: health, parent: review, status: .done)
        records.setStatus(.done, of: review)
        let store = await load()

        await store.send(.rowAction(read.id, .status(.todo)))
        await store.receive(\.delegate.saved)
        XCTAssertNotNil(store.state.banner?.undo)

        await store.send(.bannerUndoTapped)
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.workspace?.tasks[review.id]?.status, .done)
        XCTAssertNil(store.state.banner)
    }

    func testCancellingARescheduleKeepsTheDueDate() async {
        let stretch = records.task("Stretch", in: health)
        let store = await load()

        await store.send(.rowAction(stretch.id, .reschedule))
        await store.send(.reschedule(.presented(.binding(.set(\.date, now.addingTimeInterval(3 * 24 * 60 * 60))))))
        await store.send(.reschedule(.presented(.cancelTapped)))
        await store.receive(\.reschedule.dismiss)

        XCTAssertNil(store.state.reschedule)
        XCTAssertNil(store.state.workspace?.tasks[stretch.id]?.dueDate)
    }

    func testDismissingTheBannerKeepsTheChange() async {
        let review = records.task("Review", in: health)
        let read = records.task("Read", in: health, parent: review, status: .done)
        records.setStatus(.done, of: review)
        let store = await load()

        await store.send(.rowAction(read.id, .status(.todo)))
        await store.receive(\.delegate.saved)
        await store.send(.bannerDismissed)

        XCTAssertNil(store.state.banner)
        XCTAssertNotEqual(
            store.state.workspace?.tasks[review.id]?.status, .done, "Dismissing the banner undid the change")
    }
}
