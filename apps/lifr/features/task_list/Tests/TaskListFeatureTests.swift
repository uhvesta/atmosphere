import ComposableArchitecture
import Foundation
import LifrCore
import LifrFeedbackClient
import LifrPasteboardClient
import LifrStoreClient
import LifrSyncClient
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

    /// Runs a change to the store the way the feature would, at `now`.
    private func changing<T: Sendable>(_ change: () async throws -> T) async rethrows -> T {
        try await withDependencies { [now] in
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = .incrementing
        } operation: {
            try await change()
        }
    }

    private func load(
        reordering: Bool = false, searching: Bool = false, client: StoreClient? = nil, swipeTip: Bool = false
    ) async -> TestStoreOf<TaskListFeature> {
        var query = open
        query.reorders = reordering
        query.searches = searching
        var state = TaskListFeature.State(query: query)
        state.offersSwipeTip = swipeTip
        let store = TestStore(initialState: state) {
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

    func testTheSwipeTipShowsOnAListWithTasksUntilClosed() async {
        records.task("Stretch", in: health)
        let store = await load(swipeTip: true)
        XCTAssertTrue(store.state.showsSwipeTip)

        await store.send(.swipeTipClosed)

        XCTAssertFalse(store.state.showsSwipeTip)
        XCTAssertTrue(store.state.swipeTipSeen)
    }

    func testActingOnATaskCountsAsSeeingTheSwipeTip() async {
        let stretch = records.task("Stretch", in: health)
        let store = await load(swipeTip: true)

        await store.send(.rowAction(stretch.id, .status(.inProgress)))

        XCTAssertFalse(store.state.showsSwipeTip)
    }

    func testAListThatDoesntOfferTheSwipeTipNeverShowsIt() async {
        records.task("Stretch", in: health)
        let store = await load()

        XCTAssertFalse(store.state.showsSwipeTip)
    }

    func testShowsOnlySectionsWithRows() async {
        records.task("Stretch", in: health)
        records.task("Plan long runs", in: health, status: .inProgress)
        records.task("Already done", in: health, status: .done)

        let store = await load()

        XCTAssertEqual(store.state.sections.map(\.id), ["todo", "inProgress"])
    }

    func testCopyAsMarkdownPutsTheTaskOnTheClipboardWithItsCode() async throws {
        records.task("Stretch", in: health)
        let store = await load()
        let copied = LockIsolated<[String]>([])
        store.dependencies.pasteboard = PasteboardClient { text in copied.withValue { $0.append(text) } }
        let row = try XCTUnwrap(store.state.rows.first)

        await store.send(.copyTapped(row.id))
        await store.finish()

        XCTAssertEqual(copied.value, ["- [ ] Stretch (\(row.fullCode))"])
        XCTAssertTrue(row.fullCode.hasPrefix("HEALTH-"))
        @Shared(.lastCopy) var lastCopy
        XCTAssertNotNil(lastCopy, "Copying says it worked")
    }

    func testCopyListPutsEachRowUnderItsSectionsHeading() async throws {
        records.task("Stretch", in: health)
        records.task("Plan long runs", in: health, status: .inProgress)
        let store = await load()
        let copied = LockIsolated<[String]>([])
        store.dependencies.pasteboard = PasteboardClient { text in copied.withValue { $0.append(text) } }
        let titles = store.state.sections.map(\.title)
        let rows = store.state.sections.map { try? XCTUnwrap($0.rows.first) }

        await store.send(.copyListTapped)
        await store.finish()

        let text = try XCTUnwrap(copied.value.first)
        XCTAssertEqual(copied.value.count, 1)
        var at = text.startIndex
        for (title, row) in zip(titles, rows) {
            let line = try XCTUnwrap(row?.markdown)
            let heading = try XCTUnwrap(text.range(of: "## \(title)", range: at..<text.endIndex))
            let next = try XCTUnwrap(text.range(of: line, range: heading.upperBound..<text.endIndex))
            at = next.upperBound
        }
        XCTAssertTrue(text.contains("- [ ] Stretch"))
        @Shared(.lastCopy) var lastCopy
        XCTAssertNotNil(lastCopy, "Copying says it worked")
    }

    func testCopyListOnAnEmptyListCopiesNothing() async {
        let store = await load()
        let copied = LockIsolated<[String]>([])
        store.dependencies.pasteboard = PasteboardClient { text in copied.withValue { $0.append(text) } }

        await store.send(.copyListTapped)
        await store.finish()

        XCTAssertEqual(copied.value, [])
        @Shared(.lastCopy) var lastCopy
        XCTAssertNil(lastCopy, "Nothing copied, so nothing's said")
    }

    func testSearchingKeepsRowsWhoseTitleOrCodeMatchesAndDropsEmptySections() async {
        records.task("Stretch", in: health)
        let run = records.task("Plan long runs", in: health, status: .inProgress)
        let store = await load(searching: true)

        await store.send(.searchChanged("  long "))
        XCTAssertEqual(store.state.rows.map(\.title), ["Plan long runs"])
        XCTAssertEqual(store.state.sections.map(\.id), ["inProgress"])

        await store.send(.searchChanged(run.code.rawValue))
        XCTAssertEqual(store.state.rows.map(\.title), ["Plan long runs"])

        await store.send(.searchChanged("swim"))
        XCTAssertTrue(store.state.isEmpty)
        XCTAssertTrue(store.state.isSearching)

        await store.send(.searchChanged(""))
        XCTAssertEqual(store.state.rows.count, 2)
    }

    func testAListThatDoesntSearchIgnoresTheSearchText() async {
        records.task("Stretch", in: health)
        let store = await load()

        await store.send(.searchChanged("swim"))
        XCTAssertFalse(store.state.isSearching)
        XCTAssertEqual(store.state.rows.map(\.title), ["Stretch"])
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

    func testMovingARowOnePlaceDownAndBackUpWithoutDragging() async {
        for title in ["One", "Two", "Three"] { records.task(title, in: health) }
        let store = await load(reordering: true)
        await store.send(.sortPicked(.manual))
        let one = store.state.rows[0].id

        await store.send(.movedOnePlace(one, up: false))
        await store.receive(\.moved)
        XCTAssertEqual(store.state.rows.map(\.title), ["Two", "One", "Three"])
        await store.receive(\.saved)

        await store.send(.movedOnePlace(one, up: true))
        await store.receive(\.moved)
        XCTAssertEqual(store.state.rows.map(\.title), ["One", "Two", "Three"])
        await store.receive(\.saved)

        // Already at the top.
        await store.send(.movedOnePlace(one, up: true))
        XCTAssertEqual(store.state.rows.map(\.title), ["One", "Two", "Three"])
    }

    func testTrashingOneRowByNameTrashesOnlyIt() async {
        records.task("Run", in: health)
        records.task("Swim", in: health)
        let store = await load()
        let ids = store.state.rows.map(\.id)

        await store.send(.trashRowTapped(ids[0]))
        await store.receive(\.bulkSaved)
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.rows.map(\.id), [ids[1]])
    }

    func testADragThatCantBeSavedPutsTheRowBack() async {
        for title in ["One", "Two", "Three"] { records.task(title, in: health) }
        var client = StoreClient.inMemory(records.workspace)
        client.save = { _ in throw LifrError.notFound(.task, "LIFE-3") }
        let store = await load(reordering: true, client: client)
        await store.send(.sortPicked(.manual))

        await store.send(.moved(section: "todo", from: [2], to: 0))
        await store.receive(\.saved)

        XCTAssertEqual(store.state.rows.map(\.title), ["One", "Two", "Three"])
        XCTAssertNotNil(store.state.alert)
    }

    func testDroppingATaskOntoAnotherMakesItASubTask() async throws {
        let plan = records.task("Plan long runs", in: health)
        let run = records.task("Long run", in: health)
        let client = StoreClient.inMemory(records.workspace)
        let store = await load(client: client)

        await store.send(.droppedOnto(plan.id, tasks: [run.id]))
        await store.receive(\.saved)

        let saved = try await client.load()
        XCTAssertEqual(saved.tasks[run.id]?.parentID, plan.id)
        XCTAssertNil(store.state.alert)
    }

    func testDroppingATaskOntoItselfChangesNothing() async throws {
        let run = records.task("Long run", in: health)
        let client = StoreClient.inMemory(records.workspace)
        let store = await load(client: client)

        await store.send(.droppedOnto(run.id, tasks: [run.id]))

        let saved = try await client.load()
        XCTAssertNil(saved.tasks[run.id]?.parentID)
    }

    func testDroppingATaskOntoASubTaskSaysWhyItCant() async throws {
        let plan = records.task("Plan long runs", in: health)
        let week = records.task("Week one", in: health, parent: plan)
        let run = records.task("Long run", in: health)
        let client = StoreClient.inMemory(records.workspace)
        let store = await load(client: client)

        await store.send(.droppedOnto(week.id, tasks: [run.id]))
        await store.receive(\.saved)

        let saved = try await client.load()
        XCTAssertNil(saved.tasks[run.id]?.parentID)
        XCTAssertTrue(store.state.alert != nil || store.state.prompt != nil)
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

    func testDoneFeelsLikeSuccessAnotherStatusLikeATapAndOpeningLikeNothing() async {
        let stretch = records.task("Stretch", in: health)
        let run = records.task("Run", in: health)
        let store = await load()
        let (feedback, played) = FeedbackClient.recording()
        store.dependencies.feedback = feedback

        await store.send(.rowTapped(stretch.id))
        XCTAssertEqual(played.value, [])
        await store.send(.rowAction(stretch.id, .status(.done)))
        await store.receive(\.delegate.saved)
        await store.send(.rowAction(run.id, .status(.inProgress)))
        await store.receive(\.delegate.saved)
        XCTAssertEqual(played.value, [.success, .tap])
    }

    func testAClosedTaskStaysInPlaceForAMomentEvenThroughASync() async {
        let stretch = records.task("Stretch", in: health)
        let run = records.task("Run", in: health)
        let store = await load()
        let clock = TestClock()
        store.dependencies.continuousClock = clock
        store.dependencies.closedRowPause = .seconds(1.5)
        let before = store.state.rows.map(\.id)

        await store.send(.rowAction(stretch.id, .status(.done)))
        await store.receive(\.saved)
        XCTAssertEqual(store.state.rows.map(\.id), before)
        XCTAssertEqual(store.state.rows.first { $0.id == stretch.id }?.status, .done)

        // A sync's records arrive during the pause.
        if let snapshot = store.state.snapshot { await store.send(.show(snapshot)) }
        XCTAssertEqual(store.state.rows.map(\.id), before)

        await clock.advance(by: .seconds(1.5))
        await store.receive(\.lingerEnded)
        XCTAssertEqual(store.state.rows.map(\.id), [run.id])
    }

    func testScrollingEndsThePauseAtOnce() async {
        let stretch = records.task("Stretch", in: health)
        let run = records.task("Run", in: health)
        let store = await load()
        let clock = TestClock()
        store.dependencies.continuousClock = clock
        store.dependencies.closedRowPause = .seconds(1.5)

        await store.send(.rowAction(stretch.id, .status(.done)))
        await store.receive(\.saved)
        await store.send(.scrolled)
        XCTAssertEqual(store.state.rows.map(\.id), [run.id])

        await clock.advance(by: .seconds(1.5))
        await store.receive(\.lingerEnded)
        XCTAssertEqual(store.state.rows.map(\.id), [run.id])
    }

    func testWithoutAPauseAClosedTaskLeavesAtOnce() async {
        let stretch = records.task("Stretch", in: health)
        let run = records.task("Run", in: health)
        let store = await load()

        await store.send(.rowAction(stretch.id, .status(.done)))
        await store.receive(\.saved)
        XCTAssertEqual(store.state.rows.map(\.id), [run.id])
        XCTAssertEqual(store.state.lingering, [])
    }

    func testAFailedLoadShowsWhyAndTriesAgain() async {
        records.task("Stretch", in: health)
        let attempts = LockIsolated(0)
        var client = StoreClient.inMemory(records.workspace)
        let load = client.load
        client.load = {
            attempts.withValue { $0 += 1 }
            if attempts.value == 1 { throw LifrError.notFound(.task, "LIFE-1") }
            return try await load()
        }
        let store = await self.load(client: client)
        XCTAssertNotNil(store.state.loadError)
        XCTAssertNil(store.state.workspace)

        await store.send(.retryTapped)
        await store.receive(\.loaded)
        XCTAssertNil(store.state.loadError)
        XCTAssertEqual(store.state.rows.map(\.title), ["Stretch"])
    }

    func testRecordsArrivingAfterAFailedLoadClearTheError() async {
        records.task("Stretch", in: health)
        var client = StoreClient.inMemory(records.workspace)
        client.load = { throw LifrError.notFound(.task, "LIFE-1") }
        let store = await load(client: client)
        XCTAssertNotNil(store.state.loadError)

        await store.send(.show(Snapshot(records.workspace, number: 1)))

        XCTAssertNil(store.state.loadError)
        XCTAssertEqual(store.state.rows.map(\.title), ["Stretch"])
    }

    func testOpeningATaskWhilePickingEndsPickingAndOpensIt() async {
        let stretch = records.task("Stretch", in: health)
        records.task("Plan long runs", in: health)
        let store = await load()
        await store.send(.selectTapped)

        await store.send(.taskOpened(stretch.id))
        await store.receive(\.rowTapped)

        XCTAssertFalse(store.state.isSelecting)
        XCTAssertEqual(store.state.selection, [])
        XCTAssertEqual(store.state.detail?.id, stretch.id)
    }

    func testPullingSyncsAndSaysWhySyncCantRunOnlyOnce() async {
        let store = await load()
        let syncs = LockIsolated(0)
        var sync = SyncClient.off
        sync.sync = {
            syncs.withValue { $0 += 1 }
            return SyncState(status: .failed(.noAccount))
        }
        store.dependencies.sync = sync

        await store.send(.pulled)
        await store.receive(\.pullFailed)
        XCTAssertNotNil(store.state.alert)
        await store.send(.alert(.dismiss))

        await store.send(.pulled)
        await store.receive(\.pullFailed)
        XCTAssertNil(store.state.alert)
        XCTAssertEqual(syncs.value, 2)
    }

    func testOnlyAPullThatSyncsCounts() async {
        let store = await load()
        let results = LockIsolated<[SyncState.Status]>([.idle, .failed(.quotaExceeded), .idle])
        var sync = SyncClient.off
        sync.sync = { SyncState(status: results.withValue { $0.removeFirst() }) }
        store.dependencies.sync = sync
        var counts: [Int] = []

        for _ in 0..<3 {
            await store.send(.pulled)
            await store.skipReceivedActions()
            if store.state.alert != nil { await store.send(.alert(.dismiss)) }
            counts.append(store.state.syncs)
        }

        XCTAssertEqual(counts, [1, 1, 2])
    }

    func testPullingTellsASyncFailureAgainAfterASyncWorksOrItFailsAnotherWay() async {
        let store = await load()
        let results = LockIsolated<[SyncState.Status]>([
            .failed(.quotaExceeded), .idle, .failed(.quotaExceeded), .failed(.unreachable("offline")),
        ])
        var sync = SyncClient.off
        sync.sync = { SyncState(status: results.withValue { $0.removeFirst() }) }
        store.dependencies.sync = sync
        var told: [Bool] = []

        for fails in [true, false, true, true] {
            await store.send(.pulled)
            if fails { await store.receive(\.pullFailed) } else { await store.receive(\.pullSucceeded) }
            told.append(store.state.alert != nil)
            if store.state.alert != nil { await store.send(.alert(.dismiss)) }
        }

        XCTAssertEqual(told, [true, false, true, true])
    }

    func testARefusedChangeWarns() async {
        let review = records.task("Review", in: health)
        records.task("Read", in: health, parent: review)
        let store = await load()
        let (feedback, played) = FeedbackClient.recording()
        store.dependencies.feedback = feedback

        await store.send(.rowAction(review.id, .status(.done)))
        await store.receive(\.saved)
        await store.finish()
        XCTAssertEqual(played.value, [.warning])
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

    func testRescheduleMovesATaskThatRunsOverDaysAndNoDueDateClearsItsTime() async throws {
        let trip = records.task("Pack", in: health, due: 2, starts: 1)
        let call = records.task("Call", in: health, due: 1, at: "15:00")
        let store = await load()

        await store.send(.rowAction(trip.id, .reschedule))
        await store.send(.reschedule(.presented(.binding(.set(\.date, now.addingTimeInterval(4 * 24 * 60 * 60))))))
        await store.send(.reschedule(.presented(.saveTapped)))
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.workspace?.tasks[trip.id]?.startDate, try CalendarDate("2026-11-06"))
        XCTAssertEqual(store.state.workspace?.tasks[trip.id]?.dueDate, try CalendarDate("2026-11-07"))

        await store.send(.rowAction(call.id, .reschedule))
        await store.send(.reschedule(.presented(.clearTapped)))
        await store.skipReceivedActions(strict: false)
        XCTAssertNil(store.state.workspace?.tasks[call.id]?.dueDate)
        XCTAssertNil(store.state.workspace?.tasks[call.id]?.dueTime)
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

    func testClickingTheOpenRowKeepsWhatsTypedAndAnotherRowSavesItFirst() async throws {
        let stretch = records.task("Stretch", in: health)
        let run = records.task("Long run", in: health)
        let store = await load()
        await store.send(.rowTapped(stretch.id))
        await store.send(.detail(.presented(.editDetailsTapped)))
        await store.send(.detail(.presented(.binding(.set(\.detailsDraft, "Ten minutes")))))
        await store.send(.detail(.presented(.binding(.set(\.title, "Stretch calves")))))

        await store.send(.rowTapped(stretch.id))
        XCTAssertEqual(store.state.detail?.detailsDraft, "Ten minutes")

        await store.send(.rowTapped(run.id))
        await store.receive(\.saved)
        XCTAssertEqual(store.state.detail?.id, run.id)
        let saved = try await store.dependencies.store.load()
        XCTAssertEqual(saved.tasks[stretch.id]?.details, "Ten minutes")
        XCTAssertEqual(saved.tasks[stretch.id]?.title, "Stretch calves")
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

    func testRescheduleShortcutsCountFromToday() throws {
        // Tuesday.
        let today = try CalendarDate("2026-11-03")
        XCTAssertEqual(RescheduleFeature.Shortcut.today.day(from: today), today)
        XCTAssertEqual(RescheduleFeature.Shortcut.tomorrow.day(from: today), try CalendarDate("2026-11-04"))
        XCTAssertEqual(RescheduleFeature.Shortcut.thisWeekend.day(from: today), try CalendarDate("2026-11-07"))
        XCTAssertEqual(RescheduleFeature.Shortcut.nextWeek.day(from: today), try CalendarDate("2026-11-09"))
        let saturday = try CalendarDate("2026-11-07")
        let sunday = try CalendarDate("2026-11-08")
        let monday = try CalendarDate("2026-11-09")
        XCTAssertEqual(RescheduleFeature.Shortcut.thisWeekend.day(from: saturday), saturday)
        XCTAssertEqual(RescheduleFeature.Shortcut.thisWeekend.day(from: sunday), sunday)
        XCTAssertEqual(RescheduleFeature.Shortcut.nextWeek.day(from: sunday), monday)
        XCTAssertEqual(RescheduleFeature.Shortcut.nextWeek.day(from: monday), try CalendarDate("2026-11-16"))
    }

    func testAShortcutOrClearSavesInOneTap() async throws {
        let stretch = records.task("Stretch", in: health, due: 0)
        let store = await load()

        await store.send(.rowAction(stretch.id, .reschedule))
        await store.send(.reschedule(.presented(.shortcutPicked(.thisWeekend))))
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.workspace?.tasks[stretch.id]?.dueDate, try CalendarDate("2026-11-07"))
        XCTAssertNil(store.state.reschedule)

        await store.send(.rowAction(stretch.id, .reschedule))
        await store.send(.reschedule(.presented(.clearTapped)))
        await store.receive(\.delegate.saved)
        XCTAssertNil(store.state.workspace?.tasks[stretch.id]?.dueDate)
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

    // MARK: - Picking several tasks

    func testWhilePickingATapPicksTheRowInsteadOfOpeningIt() async {
        let run = records.task("Run", in: health)
        let store = await load()

        await store.send(.selectTapped)
        await store.send(.rowTapped(run.id))
        await store.receive(\.selectionToggled)

        XCTAssertEqual(store.state.selection, [run.id])
        XCTAssertNil(store.state.detail)

        await store.send(.toggled(run.id))
        await store.receive(\.selectionToggled)
        XCTAssertEqual(store.state.selection, [])
        XCTAssertNil(store.state.statusPicker)
    }

    func testPickedTasksChangeTogetherAndStayPickedForTheNextEdit() async throws {
        let run = records.task("Run", in: health)
        let swim = records.task("Swim", in: health)
        let ride = records.task("Ride", in: health)
        let client = StoreClient.inMemory(records.workspace)
        let store = await load(client: client)

        await store.send(.selectTapped)
        await store.send(.selectionToggled(run.id))
        await store.send(.selectionToggled(ride.id))
        await store.send(.bulk(.priority(.high)))
        await store.receive(\.bulkSaved)
        await store.receive(\.delegate.saved)

        let tasks = try await client.load().tasks
        XCTAssertEqual(tasks[run.id]?.priority, .high)
        XCTAssertEqual(tasks[ride.id]?.priority, .high)
        XCTAssertEqual(tasks[swim.id]?.priority, swim.priority)
        XCTAssertTrue(store.state.isSelecting)
        XCTAssertEqual(store.state.selection, [run.id, ride.id])

        await store.send(.bulk(.status(.inProgress)))
        await store.receive(\.bulkSaved)
        await store.receive(\.delegate.saved)
        XCTAssertFalse(store.state.isSelecting)
        XCTAssertEqual(store.state.selection, [])
    }

    func testArrowsMoveTheCursorAndStopAtTheEnds() async {
        records.task("A", in: health)
        records.task("B", in: health)
        records.task("C", in: health)
        let store = await load()
        let ids = store.state.rows.map(\.id)

        await store.send(.cursorMoved(up: false, extending: false))
        XCTAssertEqual(store.state.cursor, ids[0])
        await store.send(.cursorMoved(up: true, extending: false))
        XCTAssertEqual(store.state.cursor, ids[0])
        await store.send(.cursorMoved(up: false, extending: false))
        await store.send(.cursorMoved(up: false, extending: false))
        await store.send(.cursorMoved(up: false, extending: false))
        XCTAssertEqual(store.state.cursor, ids[2])
        XCTAssertFalse(store.state.isSelecting)

        await store.send(.cursorMoved(up: true, extending: true))
        XCTAssertEqual(store.state.cursor, ids[1])
        XCTAssertEqual(store.state.selection, [ids[1], ids[2]])

        await store.send(.openCursorTapped)
        await store.receive(\.rowTapped)
    }

    func testTheTrashKeyTrashesThePickedTasksAsOneChange() async throws {
        records.task("Run", in: health)
        records.task("Swim", in: health)
        records.task("Ride", in: health)
        let client = StoreClient.inMemory(records.workspace)
        let store = await load(client: client)
        let ids = store.state.rows.map(\.id)

        await store.send(.cursorMoved(up: false, extending: false))
        await store.send(.cursorMoved(up: false, extending: true))
        await store.send(.trashKeyPressed)
        await store.receive(\.bulkSaved)
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.rows.map(\.id), [ids[2]])
        XCTAssertEqual(store.state.cursor, ids[2])

        let journal = try await client.load().journal
        let entry = try XCTUnwrap(journal.last?.id)
        _ = try await changing { try await client.revert(entries: [entry]) }
        let tasks = try await client.load().tasks
        XCTAssertEqual(ids.filter { tasks[$0]?.deletedAt == nil }, ids)
    }

    func testTheKeyboardRowMovesToTheNextRowWhenItsTaskIsTrashed() async {
        records.task("A", in: health)
        records.task("B", in: health)
        records.task("C", in: health)
        let store = await load()
        let ids = store.state.rows.map(\.id)

        await store.send(.cursorMoved(up: false, extending: false))
        await store.send(.cursorMoved(up: false, extending: false))
        await store.send(.trashKeyPressed)
        await store.receive(\.bulkSaved)
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.cursor, ids[2])

        await store.send(.trashKeyPressed)
        await store.receive(\.bulkSaved)
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.cursor, ids[0])

        await store.send(.trashKeyPressed)
        await store.receive(\.bulkSaved)
        await store.receive(\.delegate.saved)
        XCTAssertNil(store.state.cursor)
    }

    func testOneUndoRevertsTheWholeChange() async throws {
        let run = records.task("Run", in: health)
        let swim = records.task("Swim", in: health)
        let client = StoreClient.inMemory(records.workspace)
        let store = await load(client: client)

        await store.send(.selectTapped)
        await store.send(.selectAllTapped)
        await store.send(.bulk(.trash))
        await store.receive(\.bulkSaved)
        await store.receive(\.delegate.saved)
        XCTAssertTrue(store.state.isEmpty)

        let journal = try await client.load().journal
        let entry = try XCTUnwrap(journal.last?.id)
        _ = try await changing { try await client.revert(entries: [entry]) }
        let tasks = try await client.load().tasks
        XCTAssertNil(tasks[run.id]?.deletedAt)
        XCTAssertNil(tasks[swim.id]?.deletedAt)
    }

    func testDueDaysCountFromToday() async throws {
        let run = records.task("Run", in: health)
        let client = StoreClient.inMemory(records.workspace)
        let store = await load(client: client)

        await store.send(.selectionToggled(run.id))
        await store.send(.bulkDue(days: 1))
        await store.receive(\.bulkSaved)

        let due = try await client.load().tasks[run.id]?.dueDate
        XCTAssertEqual(due, try CalendarDate("2026-11-04"))
    }

    func testClosingTasksWithOpenSubtasksAsksFirstAndChangesNothingUntilAnswered() async throws {
        let race = records.task("Race", in: health)
        records.task("Pin the bib", in: health, parent: race)
        let run = records.task("Run", in: health)
        let client = StoreClient.inMemory(records.workspace)
        let store = await load(client: client)

        await store.send(.selectTapped)
        await store.send(.selectionToggled(race.id))
        await store.send(.selectionToggled(run.id))
        await store.send(.bulk(.status(.done)))
        await store.receive(\.bulkSaved)

        XCTAssertNotNil(store.state.bulkPrompt)
        XCTAssertTrue(store.state.isSelecting)
        let before = try await client.load().tasks
        XCTAssertEqual(before[run.id]?.status, .todo)

        await store.send(.bulkPrompt(.presented(.closeSubtasks(.done))))
        await store.receive(\.bulkSaved)
        let after = try await client.load().tasks
        XCTAssertEqual(after[run.id]?.status, .done)
        XCTAssertEqual(after[race.id]?.status, .done)
    }

    func testShiftClickPicksTheRowsBetween() async {
        let first = records.task("A", in: health)
        records.task("B", in: health)
        let third = records.task("C", in: health)
        let store = await load()
        let ids = store.state.rows.map(\.id)

        await store.send(.selectionToggled(first.id))
        await store.send(.selectionExtended(to: third.id))

        XCTAssertTrue(store.state.isSelecting)
        XCTAssertEqual(store.state.selectedIDs, ids)
    }

    func testCommandClickWithATaskOpenPicksThatOneToo() async {
        let run = records.task("Run", in: health)
        let swim = records.task("Swim", in: health)
        let store = await load()

        await store.send(.rowTapped(run.id))
        await store.send(.selectionToggled(swim.id))

        XCTAssertEqual(store.state.selection, [run.id, swim.id])
    }

    func testAPickedTaskThatLeavesTheListIsNoLongerPicked() async throws {
        let run = records.task("Run", in: health)
        let swim = records.task("Swim", in: health)
        let client = StoreClient.inMemory(records.workspace)
        let store = await load(client: client)
        await store.send(.selectTapped)
        await store.send(.selectAllTapped)

        let saved = try await changing { try await client.setStatus(.done, run.id) }
        await store.send(.show(saved.snapshot))

        XCTAssertEqual(store.state.selection, [swim.id])
    }
}
