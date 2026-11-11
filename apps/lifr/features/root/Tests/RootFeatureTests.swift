import ComposableArchitecture
import Foundation
import LifrAPI
import LifrAvenue
import LifrCore
import LifrLockClient
import LifrModelsClient
import LifrQuickAdd
import LifrRemindersClient
import LifrRoot
import LifrSessionClient
import LifrStoreClient
import LifrSyncClient
import LifrSyncReview
import XCTest

@MainActor
final class RootFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        // Opening adds the Inbox when there's none; most tests start after.
        _ = records.inbox()
        return records
    }()
    private lazy var health = records.avenue("HEALTH", "Health")

    private let request = ApprovalRequest(
        id: UUID(sequence: 0), identity: "com.anthropic.claude-code", client: "Claude Code", method: .searchTasks,
        requestedAt: Date(timeIntervalSince1970: 1_793_700_000))

    private func makeStore(
        _ platform: RootFeature.Platform,
        store client: StoreClient? = nil,
        session: InMemorySession = InMemorySession(),
        reminders: InMemoryReminders = RemindersClient.inMemory(),
        sync: SyncClient = .off
    ) -> TestStoreOf<RootFeature> {
        let client = client ?? .inMemory(records.workspace)
        let store = TestStore(initialState: RootFeature.State(platform: platform)) {
            RootFeature()
        } withDependencies: { [now] in
            $0.store = client
            if platform == .mac { $0.session = session.client }
            $0.reminders = reminders.client
            $0.sync = sync
            $0.continuousClock = TestClock()
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = .incrementing
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
        }
        store.exhaustivity = .off
        return store
    }

    func testLoadingShowsTheRecordsOnEveryScreenAndSchedulesTheirReminders() async {
        let run = records.task("Run", in: health, due: 0)
        records.task("Stretch", in: health, due: 3)
        records.reminder(.at(now + 60 * 60), on: run)
        let reminders = RemindersClient.inMemory()
        let store = makeStore(.phone, reminders: reminders)

        let task = await store.send(.task)
        await store.receive(\.loaded)

        XCTAssertEqual(store.state.today.list.rows.map(\.title), ["Run"])
        XCTAssertEqual(store.state.upcoming.list.rows.map(\.title), ["Run", "Stretch"])
        XCTAssertEqual(store.state.avenues.avenues.map(\.name), ["Inbox", "Health"])
        XCTAssertEqual(store.state.settings.avenues.map(\.name), ["Inbox", "Health"])
        await store.receive(\.remindersScheduled)
        XCTAssertEqual(reminders.scheduled.map(\.taskID), [run.id])
        await task.cancel()
    }

    func testTheIPhoneOpensWhereThePersonWas() async {
        let stretch = records.task("Stretch", in: health, due: 3)
        @Shared(.place) var place
        $place.withLock { $0 = Place(tab: .upcoming, task: stretch.id) }
        let store = makeStore(.phone)

        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.receive(\.reopen)
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(store.state.tab, .upcoming)
        XCTAssertEqual(store.state.upcoming.list.detail?.id, stretch.id)
        await task.cancel()
    }

    func testTheMacOpensTheAvenueAndTaskThePersonHadOpen() async {
        let run = records.task("Run", in: health)
        @Shared(.place) var place
        $place.withLock { $0 = Place(sidebar: .scope(.avenue(health.id)), task: run.id) }
        let store = makeStore(.mac)

        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.receive(\.selected)
        await store.receive(\.reopen)
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(store.state.selection, .scope(.avenue(health.id)))
        XCTAssertEqual(store.state.avenue?.list.detail?.id, run.id)
        await task.cancel()
    }

    func testATaskDeletedSinceIsntOpened() async {
        @Shared(.place) var place
        $place.withLock { $0 = Place(tab: .today, task: UUID()) }
        let store = makeStore(.phone)

        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.skipReceivedActions(strict: false)

        XCTAssertNil(store.state.today.list.detail)
        await task.cancel()
    }

    func testWhereThePersonGoesIsKept() async {
        let run = records.task("Run", in: health, due: 0)
        let store = makeStore(.phone)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.today(.list(.rowTapped(run.id))))
        @Shared(.place) var place

        XCTAssertEqual(place, Place(tab: .today, task: run.id))
        await store.send(.binding(.set(\.tab, .search)))
        XCTAssertEqual(place, Place(tab: .search))
        await task.cancel()
    }

    func testTheIPhoneDoesNotUseTheSession() async {
        // The session's test value fails any call, so this passes only if
        // the iPhone never asks it.
        let store = makeStore(.phone)

        let task = await store.send(.task)
        await store.receive(\.loaded)
        await task.cancel()
    }

    func testAChangeOnOneScreenShowsOnTheOthers() async {
        let run = records.task("Run", in: health, due: 0)
        let store = makeStore(.phone)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.today(.list(.rowAction(run.id, .status(.done)))))
        await store.receive(\.today.delegate.saved)

        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.status, .done)
        XCTAssertEqual(store.state.avenues.avenues.first?.openCount, 0)
        XCTAssertEqual(store.state.search.list.workspace?.tasks[run.id]?.status, .done)
        await task.cancel()
    }

    func testSavesFromTwoScreensThatReplyOutOfOrderEndWithTheLaterSave() async throws {
        let run = records.task("Run", in: health, due: 0)
        let stretch = records.task("Stretch", in: health, due: 3)
        let replies = HeldReplies()
        var held = replies.held.makeAsyncIterator()
        let store = makeStore(.phone, store: replies.client(.inMemory(records.workspace)))
        let task = await store.send(.task)
        await store.receive(\.loaded)

        // Run is ticked off on Today, then Stretch on Upcoming.
        await store.send(.today(.list(.rowAction(run.id, .status(.done)))))
        guard let first = await held.next() else { return XCTFail("the save never ran") }
        await store.send(.upcoming(.list(.rowAction(stretch.id, .status(.done)))))
        guard let second = await held.next() else { return XCTFail("the save never ran") }
        XCTAssertLessThan(first, second)

        // Upcoming's save replies first.
        replies.release(second)
        await store.receive(\.upcoming.list.delegate.saved)
        replies.release(first)
        await store.receive(\.today.list.saved)
        await store.skipReceivedActions(strict: false)

        let workspaces = [store.state.workspace, store.state.today.list.workspace, store.state.upcoming.list.workspace]
        for workspace in workspaces {
            XCTAssertEqual(workspace?.tasks[run.id]?.status, .done)
            XCTAssertEqual(workspace?.tasks[stretch.id]?.status, .done)
        }
        XCTAssertEqual(store.state.today.list.rows.map(\.title), [])
        XCTAssertEqual(store.state.toast.map { Set($0.change.previous.keys) }, Set([stretch.id]))
        await task.cancel()
    }

    func testASaveThatRepliesAfterASyncUpdateKeepsTheUpdate() async throws {
        let run = records.task("Run", in: health, due: 0)
        let stretch = records.task("Stretch", in: health, due: 0)
        var mac = records
        mac.setStatus(.inProgress, of: stretch)
        let stretchOnMac = try XCTUnwrap(mac.workspace.tasks[stretch.id])
        let (client, sync) = StoreClient.inMemorySyncing(records.workspace)
        let replies = HeldReplies()
        var held = replies.held.makeAsyncIterator()
        let store = makeStore(.phone, store: replies.client(client))
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.today(.list(.rowAction(run.id, .status(.done)))))
        guard let save = await held.next() else { return XCTFail("the save never ran") }
        // The Mac's change arrives after the save, before its reply.
        let update = sync(now: now) { $0.applySynced(tasks: [stretchOnMac]) }
        XCTAssertLessThan(save, update.number)
        await store.receive(\.updated)
        replies.release(save)
        await store.receive(\.today.list.saved)
        await store.skipReceivedActions(strict: false)

        for workspace in [store.state.workspace, store.state.today.list.workspace] {
            XCTAssertEqual(workspace?.tasks[run.id]?.status, .done)
            XCTAssertEqual(workspace?.tasks[stretch.id]?.status, .inProgress, "the update stays")
        }
        XCTAssertEqual(store.state.toast.map { Set($0.change.previous.keys) }, Set([run.id]))
        await task.cancel()
    }

    func testAnUndoThatRepliesAfterASyncUpdateKeepsTheUpdate() async throws {
        let run = records.task("Run", in: health, due: 0)
        let stretch = records.task("Stretch", in: health, due: 0)
        var mac = records
        mac.setStatus(.inProgress, of: stretch)
        let stretchOnMac = try XCTUnwrap(mac.workspace.tasks[stretch.id])
        let (client, sync) = StoreClient.inMemorySyncing(records.workspace)
        let replies = HeldReplies()
        var held = replies.held.makeAsyncIterator()
        let store = makeStore(.phone, store: replies.client(client))
        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.today(.list(.rowAction(run.id, .status(.done)))))
        guard let save = await held.next() else { return XCTFail("the save never ran") }
        replies.release(save)
        await store.receive(\.today.delegate.saved)
        let toast = try XCTUnwrap(store.state.toast)

        await store.send(.undoTapped(toast.id))
        guard let undo = await held.next() else { return XCTFail("the undo never ran") }
        // The Mac's change arrives after the undo, before its reply.
        let update = sync(now: now) { $0.applySynced(tasks: [stretchOnMac]) }
        XCTAssertLessThan(undo, update.number)
        await store.receive(\.updated)
        replies.release(undo)
        await store.receive(\.undone)
        await store.skipReceivedActions(strict: false)

        for workspace in [store.state.workspace, store.state.today.list.workspace, store.state.search.list.workspace] {
            XCTAssertEqual(workspace?.tasks[run.id]?.status, .todo, "the undo is in the update")
            XCTAssertEqual(workspace?.tasks[stretch.id]?.status, .inProgress, "the update stays")
        }
        await task.cancel()
    }

    func testASyncThatFixedSomethingShowsTheBannerUntilDismissed() async throws {
        let parent = records.task("Buy running shoes", in: health)
        // Offline, one device adds a Sub task while the other closes the parent.
        var mac = records
        let socks = mac.task("Buy socks", in: health, parent: parent)
        records.setStatus(.done, of: parent)
        let (client, sync) = StoreClient.inMemorySyncing(records.workspace)
        let store = makeStore(.phone, store: client)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        let socksOnMac = try XCTUnwrap(mac.workspace.tasks[socks.id])
        sync(now: now) { $0.applySynced(tasks: [socksOnMac]) }
        await store.receive(\.updated)

        XCTAssertEqual(store.state.syncBanner?.messages.count, 1)
        XCTAssertEqual(store.state.workspace?.tasks[parent.id]?.status, .inProgress)

        await store.send(.syncBannerDismissed)
        XCTAssertNil(store.state.syncBanner)
        await task.cancel()
    }

    func testASyncThatFixedNothingShowsNoBanner() async {
        let (client, sync) = StoreClient.inMemorySyncing(records.workspace)
        let store = makeStore(.phone, store: client)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        sync(now: now) { _ in }
        await store.receive(\.updated)

        XCTAssertNil(store.state.syncBanner)
        await task.cancel()
    }

    func testAnUnknownAppAsksAndTheSheetGoesAwayWhenItStopsWaiting() async {
        let session = InMemorySession()
        let store = makeStore(.mac, session: session)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        session.request(request)
        await store.receive(\.sessionEvent)
        XCTAssertEqual(store.state.approval?.request, request)

        session.withdraw(request.id)
        await store.receive(\.sessionEvent)
        XCTAssertNil(store.state.approval)
        await task.cancel()
    }

    func testASecondAppWaitsForTheFirstAppsSheet() async {
        let session = InMemorySession()
        let store = makeStore(.mac, session: session)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        var second = request
        second.id = UUID(sequence: 1)
        second.client = "Cursor"

        session.request(request)
        await store.receive(\.sessionEvent)
        session.request(second)
        await store.receive(\.sessionEvent)
        XCTAssertEqual(store.state.approval?.request.client, "Claude Code")

        await store.send(.approval(.presented(.denyTapped)))
        await store.receive(\.approval.presented.answered)
        await store.receive(\.approval.dismiss)
        await store.receive(\.nextApproval)

        XCTAssertEqual(store.state.approval?.request.client, "Cursor")
        await task.cancel()
    }

    func testTheMenuBarKnowsWhenAnAppIsConnected() async {
        let grant = Grant.agentDefault(client: "Claude Code", now: now)
        let session = InMemorySession(apps: [ConnectedApp(id: "com.anthropic.claude-code", grant: grant)])
        let store = makeStore(.mac, session: session)
        let task = await store.send(.task)
        await store.receive(\.appsLoaded)
        XCTAssertFalse(store.state.hasConnectedApp)

        session.use("com.anthropic.claude-code", at: now)
        await store.receive(\.sessionEvent)
        XCTAssertTrue(store.state.hasConnectedApp)

        session.use("com.anthropic.claude-code", at: now, connected: false)
        await store.receive(\.sessionEvent)
        XCTAssertFalse(store.state.hasConnectedApp)
        await task.cancel()
    }

    func testPickingAnAvenueShowsItAsAListAndABoard() async {
        records.task("Run", in: health)
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.selected(.scope(.avenue(health.id))))
        XCTAssertEqual(store.state.avenue?.list.rows.map(\.title), ["Run"])
        XCTAssertEqual(store.state.board?.list.rows.map(\.title), ["Run"])
        XCTAssertEqual(store.state.currentList, .avenue)

        await store.send(.binding(.set(\.content, .board)))
        XCTAssertEqual(store.state.currentList, .board)

        await store.send(.selected(.today))
        XCTAssertNil(store.state.avenue)
        XCTAssertNil(store.state.board)
        await task.cancel()
    }

    func testASavedFilterOpensFromTheSidebarOrTheAvenuesTab() async {
        records.task("Buy milk", in: health, priority: .high)
        records.task("Stretch", in: health)
        let urgent = records.savedFilter("Urgent", TaskFilter(priorities: [.high]))

        let mac = makeStore(.mac)
        let macTask = await mac.send(.task)
        await mac.receive(\.loaded)
        await mac.send(.selected(.savedFilter(urgent.id)))
        XCTAssertEqual(mac.state.savedFilter?.list.rows.map(\.title), ["Buy milk"])
        XCTAssertEqual(mac.state.currentList, .savedFilter)
        await mac.send(.selected(.today))
        XCTAssertNil(mac.state.savedFilter)
        await macTask.cancel()

        let phone = makeStore(.phone)
        let phoneTask = await phone.send(.task)
        await phone.receive(\.loaded)
        await phone.send(.binding(.set(\.tab, .avenues)))
        await phone.send(.savedFilterTapped(urgent.id))
        XCTAssertEqual(phone.state.savedFilter?.list.rows.map(\.title), ["Buy milk"])
        XCTAssertEqual(phone.state.currentList, .savedFilter)
        await phoneTask.cancel()
    }

    func testASavedFilterDeletedWhileOpenCloses() async {
        let urgent = records.savedFilter("Urgent", TaskFilter(priorities: [.high]))
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.selected(.savedFilter(urgent.id)))

        await store.send(.savedFilter(.presented(.editSavedFilterTapped)))
        await store.send(.savedFilter(.presented(.editor(.presented(.deleteTapped)))))
        await store.send(.savedFilter(.presented(.editor(.presented(.confirm(.presented(.delete)))))))
        await store.skipReceivedActions(strict: false)

        XCTAssertNil(store.state.savedFilter)
        XCTAssertEqual(store.state.selection, .search)
        XCTAssertEqual(store.state.workspace?.savedFilters.isEmpty, true)
        await task.cancel()
    }

    func testSavedFiltersMoveUpAndDown() async {
        let first = records.savedFilter("First", TaskFilter(text: "a"))
        let second = records.savedFilter("Second", TaskFilter(text: "b"))
        let third = records.savedFilter("Third", TaskFilter(text: "c"))
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.savedFilterMoved(third.id, up: true))
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.workspace?.sortedSavedFilters.map(\.id), [first.id, third.id, second.id])

        await store.send(.savedFilterMoved(first.id, up: false))
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.workspace?.sortedSavedFilters.map(\.id), [third.id, first.id, second.id])

        await store.send(.savedFilterMoved(third.id, up: true))
        XCTAssertEqual(store.state.workspace?.sortedSavedFilters.map(\.id), [third.id, first.id, second.id])
        await task.cancel()
    }

    func testDroppingTasksOnASidebarAvenueOrProjectMovesThem() async throws {
        let races = records.project("Races", in: health)
        let home = records.avenue("HOME", "Home")
        let run = records.task("Run", in: health, due: 0)
        let stretch = records.task("Stretch", in: health, due: 0)
        let warmUp = records.task("Warm up", in: health, parent: run)
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.tasksDropped([run.id, stretch.id], on: .scope(.project(races.id))))
        await store.receive(\.today.delegate.saved)
        let moved = try XCTUnwrap(store.state.workspace)
        XCTAssertEqual(moved.tasks[run.id]?.projectID, races.id)
        XCTAssertEqual(moved.tasks[stretch.id]?.projectID, races.id)
        XCTAssertEqual(moved.tasks[warmUp.id]?.projectID, races.id, "a Sub task follows its parent")

        await store.send(.tasksDropped([run.id], on: .scope(.avenue(home.id))))
        await store.receive(\.today.delegate.saved)
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.avenueID, home.id)
        XCTAssertNil(store.state.workspace?.tasks[run.id]?.projectID)

        await store.send(.tasksDropped([warmUp.id], on: .scope(.avenue(health.id))))
        await store.receive(\.today.list.saved.failure)
        XCTAssertEqual(store.state.workspace?.tasks[warmUp.id]?.avenueID, home.id, "a Sub task stays with its parent")
        await task.cancel()
    }

    func testDroppingTasksOnTodayOrUpcomingMakesThemDueThenKeepingTheTimeAndSpan() async throws {
        let race = records.task("Race", in: health, due: 3, at: "09:00", starts: 1)
        let run = records.task("Run", in: health, due: 0)
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.tasksDropped([race.id], on: .today))
        await store.receive(\.today.delegate.saved)
        let today = try XCTUnwrap(store.state.workspace?.tasks[race.id])
        XCTAssertEqual(today.dueDate, try CalendarDate("2026-11-03"))
        XCTAssertEqual(today.dueTime, race.dueTime)
        XCTAssertEqual(today.startDate, try CalendarDate("2026-11-01"))

        await store.send(.tasksDropped([race.id], on: .upcoming))
        await store.receive(\.today.delegate.saved)
        XCTAssertEqual(store.state.workspace?.tasks[race.id]?.dueDate, try CalendarDate("2026-11-04"))

        // Already due today: nothing to save.
        await store.send(.tasksDropped([run.id], on: .today))
        await store.send(.tasksDropped([run.id], on: .calendar))
        await task.cancel()
    }

    func testDroppingTextAddsATaskForEachLineWhereTheItemPutsIt() async throws {
        let races = records.project("Races", in: health)
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.textDropped("- Book the 10k\n\n- Buy gels", on: .scope(.project(races.id))))
        await store.receive(\.today.delegate.saved)
        let inRaces = store.state.workspace?.tasks.values.filter { $0.projectID == races.id }
        XCTAssertEqual(inRaces?.map(\.title).sorted(), ["Book the 10k", "Buy gels"])
        XCTAssertEqual(inRaces?.map(\.avenueID), [health.id, health.id])

        await store.send(.textDropped("Call the physio", on: .today))
        await store.receive(\.today.delegate.saved)
        let call = try XCTUnwrap(store.state.workspace?.tasks.values.first { $0.title == "Call the physio" })
        XCTAssertEqual(call.avenueID, Avenue.inboxID)
        XCTAssertEqual(call.dueDate, try CalendarDate("2026-11-03"))

        await store.send(.textDropped("Ignored", on: .trash))
        XCTAssertNil(store.state.workspace?.tasks.values.first { $0.title == "Ignored" })
        await task.cancel()
    }

    func testTextSentFromAnotherAppOpensQuickAddOrAddsEachLine() async throws {
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.textSent("  Renew the passport \n"))
        await store.receive(\.linkOpened)
        XCTAssertEqual(store.state.quickAdd?.title, "Renew the passport")
        await store.send(.quickAdd(.dismiss))

        await store.send(.textSent("- Pack\n- Label boxes"))
        await store.receive(\.quickAdd.presented.added.success)
        let titles = Set(store.state.quickAdd.map { $0.workspace.tasks.values.map(\.title) } ?? [])
        XCTAssertTrue(titles.isSuperset(of: ["Pack", "Label boxes"]))
        await task.cancel()
    }

    func testPastingOnAListAddsATaskForEachLineThereOrOpensQuickAdd() async throws {
        let health = health
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.selected(.scope(.avenue(health.id))))
        await store.send(.textPasted("Stretch\nFoam roll"))
        await store.receive(\.avenue.delegate.saved)
        let inHealth = store.state.workspace?.tasks.values.filter { $0.avenueID == health.id }.map(\.title)
        XCTAssertTrue(Set(inHealth ?? []).isSuperset(of: ["Stretch", "Foam roll"]))

        await store.send(.textPasted("  \n"))
        XCTAssertNil(store.state.quickAdd)

        await store.send(.selected(.logbook))
        await store.send(.textPasted("Renew the passport"))
        await store.receive(\.textSent)
        await store.receive(\.linkOpened)
        XCTAssertEqual(store.state.quickAdd?.title, "Renew the passport")
        await task.cancel()
    }

    func testLeavingAListClosesItsTaskAndLeavingTheAssistantClosesItsChat() async {
        let run = records.task("Run", in: health, due: 0)
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.today(.list(.rowTapped(run.id))))
        XCTAssertEqual(store.state.openTaskID, run.id)
        await store.send(.selected(.conversations))
        XCTAssertNil(store.state.today.list.detail)

        await store.send(.conversations(.newTapped))
        XCTAssertNotNil(store.state.conversations.chat)
        await store.send(.selected(.today))
        await store.receive(\.conversations.closed)
        XCTAssertNil(store.state.conversations.chat)
        await task.cancel()
    }

    func testNewSubtaskNeedsAnOpenTaskAndGoesUnderIt() async {
        let shoes = records.task("Buy running shoes", in: health, due: 0)
        let gait = records.task("Get a gait analysis", in: health, parent: shoes)
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.newSubtaskTapped)
        XCTAssertNil(store.state.quickAdd)

        await store.send(.today(.list(.rowTapped(shoes.id))))
        await store.send(.newSubtaskTapped)
        XCTAssertEqual(store.state.quickAdd?.parent?.id, shoes.id)
        await store.send(.quickAdd(.dismiss))

        // A Sub task's Sub tasks go to its parent.
        await store.send(.today(.list(.detail(.dismiss))))
        await store.send(.selected(.scope(.avenue(health.id))))
        await store.send(.avenue(.list(.rowTapped(gait.id))))
        await store.send(.newSubtaskTapped)
        XCTAssertEqual(store.state.quickAdd?.parent?.id, shoes.id)
        await task.cancel()
    }

    func testANewTaskStartsInTheListItWasAddedFrom() async {
        let races = records.project("Races", in: health)
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.selected(.today))
        await store.send(.newTaskTapped)
        XCTAssertEqual(store.state.quickAdd?.due, .today)
        await store.send(.quickAdd(.dismiss))

        await store.send(.selected(.scope(.project(races.id))))
        await store.send(.newTaskTapped)
        XCTAssertEqual(store.state.quickAdd?.avenueID, health.id)
        XCTAssertEqual(store.state.quickAdd?.projectID, races.id)
        XCTAssertEqual(store.state.quickAdd?.due, QuickAddFeature.Due.none)
        await store.send(.quickAdd(.dismiss))

        await store.send(.selected(.upcoming))
        await store.send(.newTaskInTapped(.avenue(health.id)))
        XCTAssertEqual(store.state.quickAdd?.avenueID, health.id)
        XCTAssertNil(store.state.quickAdd?.projectID)
        await task.cancel()
    }

    func testANewTaskFromTheCalendarIsDueOnTheSelectedDay() async throws {
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        let day = try CalendarDate("2026-11-20")

        await store.send(.selected(.calendar))
        await store.send(.calendar(.daySelected(day)))
        await store.send(.newTaskTapped)

        XCTAssertEqual(store.state.quickAdd?.due, .pick)
        XCTAssertEqual(store.state.quickAdd.map { CalendarDate($0.pickedDate, in: .gmt) }, day)
        await task.cancel()
    }

    func testATaskRestoredFromTheTrashIsBackOnToday() async {
        let run = records.task("Run", in: health, due: 0)
        records.trash(run)
        let store = makeStore(.phone)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.trash.rows.map(\.title), ["Run"])
        XCTAssertTrue(store.state.today.list.rows.isEmpty)

        await store.send(.trash(.restoreTapped(run.id)))
        await store.receive(\.trash.delegate.saved)

        XCTAssertEqual(store.state.today.list.rows.map(\.title), ["Run"])
        XCTAssertTrue(store.state.trash.rows.isEmpty)
        await task.cancel()
    }

    func testOpeningDeletesTasksTrashedLongerThanTheTrashKeepsThem() async throws {
        let old = records.task("Old", in: health)
        let recent = records.task("Recent", in: health)
        records.trash(old, daysAgo: 31)
        records.trash(recent, daysAgo: 29)
        let client = StoreClient.inMemory(records.workspace)
        let store = makeStore(.phone, store: client)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        XCTAssertEqual(store.state.trash.rows.map(\.title), ["Recent"])
        let saved = try await client.load()
        XCTAssertNil(saved.tasks[old.id])
        await task.cancel()
    }

    func testOpeningWithNoInboxAddsOneWithNothingToUndo() async throws {
        let client = StoreClient.inMemory(RecordsBuilder(now: now, timeZone: .gmt).workspace)
        let store = makeStore(.phone, store: client)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        XCTAssertEqual(store.state.avenues.avenues.map(\.id), [Avenue.inboxID])
        XCTAssertNil(store.state.toast)
        await task.cancel()
    }

    func testTheIPhonesUpcomingTabActsOnTheCalendarOnceShown() async {
        let store = makeStore(.phone)
        await store.send(.binding(.set(\.tab, .upcoming)))
        XCTAssertEqual(store.state.currentList, .upcoming)

        await store.send(.upcomingCalendarShown(true))

        XCTAssertEqual(store.state.currentList, .calendar)
    }

    func testANewProjectStartsInTheAvenueOnScreenOrTheOneAsked() async {
        let home = records.avenue("HOME", "Home")
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.selected(.scope(.avenue(home.id))))
        await store.send(.newProjectTapped(nil))
        XCTAssertEqual(store.state.quickAdd?.kind, .project)
        XCTAssertEqual(store.state.quickAdd?.avenueID, home.id)
        await store.send(.quickAdd(.dismiss))

        await store.send(.newProjectTapped(health.id))
        XCTAssertEqual(store.state.quickAdd?.avenueID, health.id)
        await task.cancel()
    }

    func testAddTaskOnABoardAddsToItsAvenue() async {
        let home = records.avenue("HOME", "Home")
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.selected(.scope(.avenue(home.id))))
        await store.send(.binding(.set(\.content, .board)))
        await store.send(.board(.addTapped))
        await store.receive(\.newTaskInTapped)
        XCTAssertEqual(store.state.quickAdd?.kind, .task)
        XCTAssertEqual(store.state.quickAdd?.avenueID, home.id)
        await task.cancel()
    }

    func testNewAvenueFromTheSidebarAsksForOne() async {
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.newAvenueTapped)
        await store.receive(\.avenues.newAvenueTapped)
        XCTAssertNotNil(store.state.avenues.newAvenue)
        await task.cancel()
    }

    func testAToastGoesAfterFiveSecondsOnTheAppsClock() async throws {
        let run = records.task("Run", in: health, due: 0)
        let store = makeStore(.phone)
        let clock = TestClock()
        store.dependencies.continuousClock = clock
        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.today(.list(.rowAction(run.id, .status(.done)))))
        await store.receive(\.today.delegate.saved)
        let toast = try XCTUnwrap(store.state.toast)

        let shown = await store.send(.toastShown(toast.id))
        await clock.advance(by: .seconds(4))
        XCTAssertNotNil(store.state.toast)
        await clock.advance(by: .seconds(1))
        await store.receive(\.toastExpired)
        XCTAssertNil(store.state.toast)
        await shown.finish()
        await task.cancel()
    }

    func testAChangeOffersUndoAndUndoRevertsIt() async throws {
        let run = records.task("Run", in: health, due: 0)
        let store = makeStore(.phone)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.today(.list(.rowAction(run.id, .status(.done)))))
        await store.receive(\.today.delegate.saved)
        let toast = try XCTUnwrap(store.state.toast)
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.status, .done)

        await store.send(.toastExpired(toast.id))
        XCTAssertNil(store.state.toast)

        // Undo still works once the toast is gone, as ⌘Z or a shake.
        await store.send(.undoTapped(toast.id))
        await store.receive(\.undone)
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.status, .todo)
        XCTAssertNil(store.state.lastChange)
        XCTAssertEqual(store.state.today.list.rows.map(\.title), ["Run"])
        await task.cancel()
    }

    func testUndoRefusesOnceTheTaskChangedAgainElsewhere() async throws {
        let run = records.task("Run", in: health, due: 0)
        let client = StoreClient.inMemory(records.workspace)
        let store = makeStore(.phone, store: client)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.today(.list(.rowAction(run.id, .status(.done)))))
        await store.receive(\.today.delegate.saved)
        let id = try XCTUnwrap(store.state.lastChange).id

        // Another device renames it.
        let synced = try await withDependencies {
            $0.uuid = UUIDGenerator { UUID() }
            $0.date.now = now
        } operation: {
            try await client.edit(run.id, Workspace.TaskEdit(title: "Run 5 km"))
        }
        await store.send(.updated(Update(workspace: synced.workspace, number: synced.number)))

        await store.send(.undoTapped(id))
        XCTAssertNotNil(store.state.notice)
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.status, .done)
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.title, "Run 5 km")
        await task.cancel()
    }

    func testFindGoesToSearchWithTheCursorInItsField() async {
        for platform in [RootFeature.Platform.mac, .phone] {
            let store = makeStore(platform)
            await store.send(.goTo(.search))
            await store.skipReceivedActions(strict: false)
            XCTAssertTrue(store.state.search.isFieldFocused)
            XCTAssertTrue(store.state.selection == .search || store.state.tab == .search)
        }
    }

    func testThePlanCommandsAreOfferedOnlyWithAConversationOnScreen() async {
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertNil(store.state.openChatID)

        await store.send(.selected(.conversations))
        await store.send(.conversations(.newTapped))
        await store.skipReceivedActions(strict: false)
        let id = store.state.openChatID
        XCTAssertNotNil(id)
        XCTAssertEqual(id, store.state.conversations.openID)

        await store.send(.selected(.today))
        XCTAssertNil(store.state.openChatID)
        await task.cancel()
    }

    func testPrintIsOfferedForAShownListThatHasTasksAndNotWhileLocked() async {
        records.task("Run", in: health, due: 0)
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        XCTAssertEqual(store.state.printable?.title, "Today")
        XCTAssertTrue(store.state.printable?.list.markdown.contains("Run") == true)

        await store.send(.selected(.scope(.avenue(health.id))))
        XCTAssertEqual(store.state.printable?.title, "Health")

        await store.send(.selected(.logbook))
        XCTAssertNil(store.state.printable, "nothing closed yet")

        await store.send(.selected(.conversations))
        XCTAssertNil(store.state.printable)
        await task.cancel()
    }

    func testAddTaskOnAnAvenueOrProjectPageOpensQuickAddThere() async {
        let marathon = records.project("Half marathon", in: health)
        let store = makeStore(.phone)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.avenues(.opened(.project(marathon.id))))
        await store.send(.avenues(.avenue(.presented(.addTaskTapped))))
        await store.receive(\.newTaskInTapped)
        XCTAssertEqual(store.state.quickAdd?.avenueID, health.id)
        XCTAssertEqual(store.state.quickAdd?.projectID, marathon.id)
        await store.send(.quickAdd(.dismiss))

        await store.send(.avenues(.avenue(.dismiss)))
        await store.send(.avenues(.opened(.avenue(health.id))))
        await store.send(.avenues(.avenue(.presented(.addTaskTapped))))
        await store.receive(\.newTaskInTapped)
        XCTAssertEqual(store.state.quickAdd?.avenueID, health.id)
        XCTAssertNil(store.state.quickAdd?.projectID)
        await task.cancel()
    }

    func testAProjectChangeOffersUndoAndRedoAndEachPutsItAndItsTasksBack() async throws {
        let marathon = records.project("Half marathon", in: health)
        let shoes = records.task("Buy running shoes", in: health, project: marathon)
        let store = makeStore(.phone)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.avenues(.opened(.project(marathon.id))))
        await store.send(.avenues(.avenue(.presented(.project(.archiveTapped)))))
        await store.send(.avenues(.avenue(.presented(.project(.archive(.presented(.moveToAvenue)))))))
        await store.skipReceivedActions()
        let toast = try XCTUnwrap(store.state.toast)
        XCTAssertEqual(store.state.workspace?.projects[marathon.id]?.status, .archived)
        XCTAssertNil(store.state.workspace?.tasks[shoes.id]?.projectID)

        await store.send(.undoTapped(toast.id))
        await store.receive(\.undone)
        XCTAssertEqual(store.state.workspace?.projects[marathon.id]?.status, .active)
        XCTAssertEqual(store.state.workspace?.tasks[shoes.id]?.projectID, marathon.id)
        XCTAssertEqual(store.state.avenues.avenue?.project?.status, .active, "the open Project shows it")
        XCTAssertNil(store.state.lastChange)

        await store.send(.redoTapped(try XCTUnwrap(store.state.redo).id))
        await store.receive(\.redone)
        XCTAssertEqual(store.state.workspace?.projects[marathon.id]?.status, .archived)
        XCTAssertNil(store.state.workspace?.tasks[shoes.id]?.projectID)
        await store.send(.undoTapped(try XCTUnwrap(store.state.lastChange).id))
        await store.receive(\.undone)
        XCTAssertEqual(store.state.workspace?.projects[marathon.id]?.status, .active)
        XCTAssertEqual(store.state.workspace?.tasks[shoes.id]?.projectID, marathon.id)
        await task.cancel()
    }

    func testMovingAProjectTakesItsTasksAndUndoPutsThemAllBack() async throws {
        let marathon = records.project("Half marathon", in: health)
        let home = records.avenue("HOME", "Home")
        let shoes = records.task("Buy running shoes", in: health, project: marathon)
        let laces = records.task("Pick laces", in: health, project: marathon, parent: shoes)
        let store = makeStore(.phone)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.avenues(.opened(.project(marathon.id))))
        await store.send(.avenues(.avenue(.presented(.project(.avenuePicked(home.id))))))
        await store.skipReceivedActions()
        let toast = try XCTUnwrap(store.state.toast)
        for id in [shoes.id, laces.id] {
            XCTAssertEqual(store.state.workspace?.tasks[id]?.avenueID, home.id)
            XCTAssertEqual(store.state.workspace?.tasks[id]?.projectID, marathon.id)
        }
        XCTAssertEqual(store.state.workspace?.projects[marathon.id]?.avenueID, home.id)

        await store.send(.undoTapped(toast.id))
        await store.receive(\.undone)
        XCTAssertEqual(store.state.workspace?.projects[marathon.id]?.avenueID, health.id)
        for id in [shoes.id, laces.id] {
            XCTAssertEqual(store.state.workspace?.tasks[id]?.avenueID, health.id)
            XCTAssertEqual(store.state.workspace?.tasks[id]?.projectID, marathon.id)
        }
        await task.cancel()
    }

    func testMovingATaskTakesItsSubtasksAndUndoPutsThemAllBack() async throws {
        let home = records.avenue("HOME", "Home")
        let garden = records.project("Garden", in: home)
        let run = records.task("Run", in: health, due: 0)
        let warmUp = records.task("Warm up", in: health, parent: run)
        let store = makeStore(.phone)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.today(.list(.rowTapped(run.id))))
        await store.send(.today(.list(.detail(.presented(.projectPicked(garden.id))))))
        await store.skipReceivedActions()
        let toast = try XCTUnwrap(store.state.toast)
        for id in [run.id, warmUp.id] {
            XCTAssertEqual(store.state.workspace?.tasks[id]?.avenueID, home.id)
            XCTAssertEqual(store.state.workspace?.tasks[id]?.projectID, garden.id)
        }

        await store.send(.undoTapped(toast.id))
        await store.receive(\.undone)
        for id in [run.id, warmUp.id] {
            XCTAssertEqual(store.state.workspace?.tasks[id]?.avenueID, health.id)
            XCTAssertNil(store.state.workspace?.tasks[id]?.projectID)
        }
        await task.cancel()
    }

    func testUndoRefusesOnceTheProjectChangedAgainElsewhere() async throws {
        let marathon = records.project("Half marathon", in: health)
        let client = StoreClient.inMemory(records.workspace)
        let store = makeStore(.phone, store: client)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.avenues(.opened(.project(marathon.id))))
        await store.send(.avenues(.avenue(.presented(.project(.binding(.set(\.name, "Spring half")))))))
        await store.send(.avenues(.avenue(.presented(.project(.nameSubmitted)))))
        await store.skipReceivedActions()
        let id = try XCTUnwrap(store.state.lastChange).id

        // Another device gives it a due date.
        let synced = try await withDependencies {
            $0.uuid = UUIDGenerator { UUID() }
            $0.date.now = now
        } operation: {
            try await client.edit(
                project: marathon.id, Workspace.ProjectEdit(dueDate: try CalendarDate(year: 2026, month: 12, day: 6)))
        }
        await store.send(.updated(Update(workspace: synced.workspace, number: synced.number)))

        await store.send(.undoTapped(id))
        XCTAssertNotNil(store.state.notice)
        XCTAssertEqual(store.state.workspace?.projects[marathon.id]?.name, "Spring half")
        await task.cancel()
    }

    func testWhatAnAvenueSharesWithAppsIsNotAChangeToUndo() async throws {
        let health = health
        let client = StoreClient.inMemory(records.workspace)

        let saved = try await withDependencies {
            $0.uuid = .incrementing
            $0.date.now = now
        } operation: {
            try await client.setAgentAccess(.readOnly, ofAvenue: health.id)
        }

        XCTAssertNil(LastChange(saved, id: UUID()))
    }

    func testAddingACommentIsNotAChangeToUndo() async {
        let run = records.task("Run", in: health, due: 0)
        let store = makeStore(.phone)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.today(.list(.rowTapped(run.id))))
        await store.send(.today(.list(.detail(.presented(.binding(.set(\.newComment, "Felt good")))))))
        await store.send(.today(.list(.detail(.presented(.commentSubmitted)))))
        await store.receive(\.today.delegate.saved)
        XCTAssertNil(store.state.lastChange)
        await task.cancel()
    }

    func testStatusShortcutsChangeTheOpenTaskOnly() async {
        let run = records.task("Run", in: health, due: 0)
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.statusShortcut(.inProgress))
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.status, .todo)

        await store.send(.today(.list(.rowTapped(run.id))))
        await store.send(.statusShortcut(.inProgress))
        await store.receive(\.today.delegate.saved)
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.status, .inProgress)
        await task.cancel()
    }

    func testRedoPutsBackWhatUndoTookAndUndoTakesItAgain() async throws {
        let run = records.task("Run", in: health, due: 0)
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.today(.list(.rowAction(run.id, .status(.done)))))
        await store.receive(\.today.delegate.saved)
        let done = try XCTUnwrap(store.state.lastChange)

        await store.send(.undoTapped(done.id))
        await store.receive(\.undone)
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.status, .todo)
        let redo = try XCTUnwrap(store.state.redo)

        await store.send(.redoTapped(redo.id))
        await store.receive(\.redone)
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.status, .done)
        XCTAssertNil(store.state.redo)
        let again = try XCTUnwrap(store.state.lastChange)
        XCTAssertEqual(again.message, done.message)

        await store.send(.undoTapped(again.id))
        await store.receive(\.undone)
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.status, .todo)
        await task.cancel()
    }

    func testRedoRefusesOnceTheTaskChangedAgainElsewhere() async throws {
        let run = records.task("Run", in: health, due: 0)
        let client = StoreClient.inMemory(records.workspace)
        let store = makeStore(.mac, store: client)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.today(.list(.rowAction(run.id, .status(.done)))))
        await store.receive(\.today.delegate.saved)
        await store.send(.undoTapped(try XCTUnwrap(store.state.lastChange).id))
        await store.receive(\.undone)
        let redo = try XCTUnwrap(store.state.redo)

        let synced = try await withDependencies {
            $0.uuid = UUIDGenerator { UUID() }
            $0.date.now = now
        } operation: {
            try await client.edit(run.id, Workspace.TaskEdit(title: "Run 5 km"))
        }
        await store.send(.updated(Update(workspace: synced.workspace, number: synced.number)))

        await store.send(.redoTapped(redo.id))
        XCTAssertNotNil(store.state.notice)
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.status, .todo)
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.title, "Run 5 km")
        await task.cancel()
    }

    func testPriorityAndDueShortcutsChangeTheOpenTask() async throws {
        let run = records.task("Run", in: health)
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.goTo(.anytime))
        await store.skipReceivedActions(strict: false)

        // Nothing open, nothing changes.
        await store.send(.priorityShortcut(.high))
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.priority, Priority.none)

        await store.send(.anytime(.rowTapped(run.id)))
        await store.skipReceivedActions(strict: false)
        await store.send(.priorityShortcut(.high))
        await store.receive(\.anytime.delegate.saved)
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.priority, .high)

        await store.send(.dueShortcut(.tomorrow))
        await store.receive(\.anytime.delegate.saved)
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.dueDate, CalendarDate(now + 86_400, in: .gmt))

        await store.send(.dueShortcut(.none))
        await store.receive(\.anytime.delegate.saved)
        XCTAssertNil(store.state.workspace?.tasks[run.id]?.dueDate)
        await task.cancel()
    }

    func testGoToShowsTheListOnEitherPlatform() async {
        let mac = makeStore(.mac)
        await mac.send(.goTo(.calendar))
        await mac.skipReceivedActions(strict: false)
        XCTAssertEqual(mac.state.selection, .calendar)

        let phone = makeStore(.phone)
        await phone.send(.goTo(.calendar))
        await phone.skipReceivedActions(strict: false)
        XCTAssertEqual(phone.state.tab, .upcoming)
        XCTAssertTrue(phone.state.upcomingShowsCalendar)
        await phone.send(.goTo(.search))
        XCTAssertEqual(phone.state.tab, .search)
    }

    func testGoToTaskFindsByCodeOrWordsAndOpensThePick() async throws {
        let run = records.task("Run 5 km", in: health, due: 0)
        let swim = records.task("Swim", in: health)
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        let workspace = try XCTUnwrap(store.state.workspace)

        await store.send(.goToTaskTapped)
        let code = workspace.displayCode(of: try workspace.task(swim.id))
        await store.send(.goToTask(.presented(.binding(.set(\.query, code.lowercased())))))
        XCTAssertEqual(store.state.goToTask?.matches.map(\.id), [swim.id])
        await store.send(.goToTask(.presented(.binding(.set(\.query, "health run")))))
        XCTAssertEqual(store.state.goToTask?.matches.map(\.id), [run.id])
        await store.send(.goToTask(.presented(.binding(.set(\.query, "nothing like it")))))
        XCTAssertEqual(store.state.goToTask?.matches, [])

        await store.send(.goToTask(.presented(.binding(.set(\.query, "run")))))
        await store.send(.goToTask(.presented(.submitted)))
        await store.skipReceivedActions(strict: false)
        XCTAssertNil(store.state.goToTask)
        XCTAssertEqual(store.state.openTaskID, run.id)
        await task.cancel()
    }

    // MARK: Sync

    /// A sync client that counts its syncs and reports `review`.
    private func counting(_ syncs: LockIsolated<Int>, review: [Review.Item] = []) -> SyncClient {
        let state = SyncState(lastSynced: now, review: review)
        return SyncClient(
            states: { AsyncStream { $0.yield(state) } },
            sync: {
                syncs.withValue { $0 += 1 }
                return state
            },
            resolve: { [now] _ in SyncState(lastSynced: now) },
            changed: { syncs.withValue { $0 += 100 } })
    }

    func testLaunchSyncsAndShowsWhatWaitsForReview() async {
        let syncs = LockIsolated(0)
        let review = Review.sample(now: now, timeZone: .gmt).items
        let store = makeStore(.phone, sync: counting(syncs, review: review))
        let task = await store.send(.task)
        await store.receive(\.syncChanged)
        XCTAssertEqual(store.state.sync.review, review)
        XCTAssertEqual(store.state.settings.sync.review, review)
        await store.skipReceivedActions()
        XCTAssertEqual(syncs.value, 1)
        await task.cancel()
    }

    func testOnceTheDayChangesTomorrowsTasksAreDueToday() async {
        records.task("Stretch", in: health, due: 1)
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.today.list.rows.map(\.title), [])

        store.dependencies.date.now = now + 24 * 60 * 60
        await store.send(.dayChanged)
        await store.receive(\.loaded)

        XCTAssertEqual(store.state.today.list.rows.map(\.title), ["Stretch"])
        XCTAssertEqual(store.state.upcoming.list.sections.filter { !$0.rows.isEmpty }.map(\.title), ["Today"])
        await task.cancel()
    }

    func testOpeningAReminderFromChatsShowsItsTaskFromToday() async {
        let stretch = records.task("Stretch", in: health, due: 3)
        let store = makeStore(.phone)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.binding(.set(\.tab, .conversations)))

        await store.send(.reminderOpened(stretch.id))
        await store.receive(\.reopen)
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(store.state.tab, .today)
        XCTAssertEqual(store.state.today.list.detail?.id, stretch.id)
        await task.cancel()
    }

    func testChangeStatusOnAReminderShowsItsTaskWithTheStatusesToPick() async {
        let stretch = records.task("Stretch", in: health, due: 3)
        let store = makeStore(.phone)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.reminderStatusRequested(stretch.id))
        await store.receive(\.reopen)
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(store.state.today.list.detail?.id, stretch.id)
        XCTAssertNotNil(store.state.today.list.detail?.statusPicker)
        await task.cancel()
    }

    func testATaskLinkFromTheAssistantTabLandsOnTheTask() async throws {
        let stretch = records.task("Stretch", in: health, due: 3)
        let store = makeStore(.phone)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.binding(.set(\.tab, .conversations)))

        await store.send(.urlOpened(try XCTUnwrap(DeepLink.task(stretch.id, picksStatus: false).url)))
        await store.receive(\.reopen)
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(store.state.tab, .today)
        XCTAssertEqual(store.state.today.list.detail?.id, stretch.id)
        await task.cancel()
    }

    func testALinkBeforeTheRecordsLoadIsFollowedOnceTheyDo() async throws {
        let stretch = records.task("Stretch", in: health, due: 3)
        let store = makeStore(.phone)

        await store.send(.urlOpened(try XCTUnwrap(URL(string: "lifr://task/\(stretch.id.uuidString)"))))
        await store.send(.urlOpened(try XCTUnwrap(URL(string: "lifr://add?title=Buy%20milk"))))
        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(store.state.today.list.detail?.id, stretch.id)
        XCTAssertEqual(store.state.quickAdd?.title, "Buy milk")
        await task.cancel()
    }

    func testALinkToATaskLifrDoesntHaveSaysSoAndGoesToToday() async throws {
        let store = makeStore(.phone)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.binding(.set(\.tab, .conversations)))

        await store.send(.linkOpened(.task(UUID(), picksStatus: false)))
        await store.skipReceivedActions(strict: false)

        XCTAssertNotNil(store.state.notice)
        XCTAssertEqual(store.state.tab, .today)
        XCTAssertNil(store.state.today.list.detail)

        await store.send(.urlOpened(try XCTUnwrap(URL(string: "https://example.com/task"))))
        XCTAssertNotNil(store.state.notice)
        await task.cancel()
    }

    func testALinkWaitsForTheLockAndIsFollowedOnceUnlocked() async throws {
        let (store, _) = locked(.always(failing: .cancelled))
        let task = await store.send(.task)
        await store.receive(\.unlocked)
        await store.skipReceivedActions(strict: false)
        XCTAssertTrue(store.state.isLocked)

        await store.send(.linkOpened(.calendar(try CalendarDate("2026-11-09"))))
        XCTAssertEqual(store.state.tab, .today)

        await store.send(.unlocked(.success(())))
        await store.receive(\.linkOpened)
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(store.state.tab, .upcoming)
        XCTAssertTrue(store.state.upcomingShowsCalendar)
        XCTAssertEqual(store.state.calendar.selected, try CalendarDate("2026-11-09"))
        await task.cancel()
    }

    func testAnAvenuesLinkOpensItsPageOnEachPlatform() async {
        let health = health
        let mac = makeStore(.mac)
        var task = await mac.send(.task)
        await mac.receive(\.loaded)
        await mac.send(.linkOpened(.avenue(health.id)))
        await mac.skipReceivedActions(strict: false)
        XCTAssertEqual(mac.state.selection, .scope(.avenue(health.id)))
        await task.cancel()

        let phone = makeStore(.phone)
        task = await phone.send(.task)
        await phone.receive(\.loaded)
        await phone.send(.linkOpened(.avenue(health.id)))
        await phone.skipReceivedActions(strict: false)
        XCTAssertEqual(phone.state.tab, .avenues)
        XCTAssertEqual(phone.state.avenues.avenue?.scope, .avenue(health.id))

        await phone.send(.linkOpened(.avenue(UUID())))
        await phone.skipReceivedActions(strict: false)
        XCTAssertEqual(phone.state.tab, .today, "an Avenue that's gone opens Today")
        await task.cancel()
    }

    func testLinksReadBackWhatTheyName() throws {
        let id = UUID()
        let links: [DeepLink] = [
            .task(id, picksStatus: false), .task(id, picksStatus: true), .today, .avenue(id),
            .calendar(try CalendarDate("2026-11-09")), .add(title: nil), .add(title: "Call mum & dad"),
        ]
        for link in links {
            XCTAssertEqual(try XCTUnwrap(link.url).absoluteString.hasPrefix("lifr://"), true)
            XCTAssertEqual(DeepLink(try XCTUnwrap(link.url)), link)
        }
        for bad in ["lifr://task/not-an-id", "lifr://calendar/9-11-2026", "lifr://nowhere", "https://lifr.app/today"] {
            XCTAssertNil(DeepLink(try XCTUnwrap(URL(string: bad))), bad)
        }
    }

    func testTheBadgeCountsWhatsDueAndFollowsTheSetting() async {
        records.task("Run", in: health, due: 0)
        records.task("Stretch", in: health, due: -1)
        records.task("Book physio", in: health, due: 3)
        let reminders = RemindersClient.inMemory()
        let store = makeStore(.phone, reminders: reminders)

        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.receive(\.remindersScheduled)
        XCTAssertEqual(reminders.badge, 2)

        store.state.$badgeCount.withLock { $0 = .overdue }
        await store.receive(\.badgeCountChanged)
        await store.receive(\.remindersScheduled)
        XCTAssertEqual(reminders.badge, 1)
        await task.cancel()
    }

    func testAReminderOpenedAtLaunchShowsItsTaskOnceTheRecordsLoad() async {
        let stretch = records.task("Stretch", in: health, due: 3)
        let store = makeStore(.mac)

        await store.send(.reminderOpened(stretch.id))
        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.receive(\.reopen)
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(store.state.selection, .today)
        XCTAssertEqual(store.state.today.list.detail?.id, stretch.id)
        await task.cancel()
    }

    func testWhileLockedRemindersDontSayWhatsDue() async {
        let run = records.task("Run", in: health, due: 0)
        records.reminder(.at(now + 60 * 60), on: run)
        let reminders = RemindersClient.inMemory()
        let (store, _) = locked(.always())
        store.dependencies.reminders = reminders.client

        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.receive(\.remindersScheduled)

        XCTAssertEqual(reminders.scheduled.map(\.taskID), [run.id])
        XCTAssertFalse(reminders.scheduled.contains { $0.title.contains("Run") || $0.body.contains("HEALTH") })
        await task.cancel()
    }

    func testComingToTheFrontSyncs() async {
        let syncs = LockIsolated(0)
        let store = makeStore(.phone, sync: counting(syncs))
        await store.send(.foregrounded)
        await store.finish()
        XCTAssertEqual(syncs.value, 1)
    }

    func testSyncingContinuouslyChecksEveryTwoMinutes() async {
        let syncs = LockIsolated(0)
        let store = makeStore(.phone, sync: counting(syncs))
        let clock = TestClock()
        store.dependencies.continuousClock = clock
        let task = await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(syncs.value, 1)

        await clock.advance(by: .seconds(2 * 60))
        XCTAssertEqual(syncs.value, 2)
        await task.cancel()
    }

    func testStartingTwiceRunsOneTimer() async {
        let syncs = LockIsolated(0)
        let store = makeStore(.mac, sync: counting(syncs))
        let clock = TestClock()
        store.dependencies.continuousClock = clock
        let first = await store.send(.task)
        await store.receive(\.loaded)
        let second = await store.send(.task)
        XCTAssertEqual(syncs.value, 1)

        await clock.advance(by: .seconds(2 * 60))
        XCTAssertEqual(syncs.value, 2)
        await first.cancel()
        await second.cancel()
    }

    func testWithContinuousSyncOffItChecksEveryFifteenMinutes() async {
        let syncs = LockIsolated(0)
        let store = makeStore(.phone, sync: counting(syncs))
        let clock = TestClock()
        store.dependencies.continuousClock = clock
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.settings(.syncContinuouslyToggled(false)))
        XCTAssertFalse(store.state.syncsContinuously)
        await clock.advance(by: .seconds(14 * 60))
        XCTAssertEqual(syncs.value, 1)
        await clock.advance(by: .seconds(60))
        XCTAssertEqual(syncs.value, 2)
        await task.cancel()
    }

    func testGoingToTheBackgroundSyncsOnlyWhileContinuous() async {
        let syncs = LockIsolated(0)
        let store = makeStore(.phone, sync: counting(syncs))
        await store.send(.backgrounded)
        await store.finish()
        XCTAssertEqual(syncs.value, 1)

        await store.send(.settings(.syncContinuouslyToggled(false)))
        await store.send(.backgrounded)
        await store.finish()
        XCTAssertEqual(syncs.value, 1)
    }

    func testSyncNowOnTheMacSyncs() async {
        let syncs = LockIsolated(0)
        let store = makeStore(.mac, sync: counting(syncs))
        await store.send(.syncNowTapped)
        await store.finish()
        XCTAssertEqual(syncs.value, 1)
    }

    func testAChangeMadeHereTellsSync() async throws {
        let syncs = LockIsolated(0)
        let run = records.task("Run", in: health, due: 0)
        let store = makeStore(.phone, sync: counting(syncs))
        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.today(.list(.rowAction(run.id, .status(.done)))))
        await store.receive(\.today.delegate.saved)
        await store.skipReceivedActions()
        await task.cancel()
        XCTAssertGreaterThanOrEqual(syncs.value, 100)
    }

    func testReviewOpensTheSheetAndDoneClosesIt() async {
        let syncs = LockIsolated(0)
        let review = Review.sample(now: now, timeZone: .gmt).items
        let store = makeStore(.mac, sync: counting(syncs, review: review))
        await store.send(.syncChanged(SyncState(review: review)))
        await store.send(.syncReviewTapped) {
            $0.syncReview = SyncReviewFeature.State(items: review, workspace: Workspace(), thisDevice: "This Mac")
        }
        await store.send(.syncReview(.presented(.doneTapped)))
        await store.receive(\.syncReview.presented.resolved)
        await store.receive(\.syncReview.presented.delegate.finished) {
            $0.syncReview = nil
        }
    }

    func testReviewWithNothingHeldOpensNothing() async {
        let store = makeStore(.phone)
        await store.send(.syncReviewTapped)
        XCTAssertNil(store.state.syncReview)
    }

    // MARK: The lock

    /// A phone store whose lock is on, confirming with `lock`, and the
    /// time it thinks it is.
    private func locked(_ lock: LockClient) -> (TestStoreOf<RootFeature>, LockIsolated<Date>) {
        let time = LockIsolated(now)
        let store = makeStore(.phone)
        store.dependencies.lock = lock
        store.dependencies.date = DateGenerator { time.value }
        store.state.$locksApp.withLock { $0 = true }
        return (store, time)
    }

    func testALockedAppAsksWhenItOpensAndShowsOnceConfirmed() async {
        let (store, _) = locked(.always())

        let task = await store.send(.task)
        XCTAssertTrue(store.state.isLocked)
        await store.receive(\.unlocked)
        XCTAssertFalse(store.state.isLocked)
        await task.cancel()
    }

    func testMenusAndShortcutsWaitWhileLocked() async {
        let (store, _) = locked(.always(failing: .cancelled))

        let task = await store.send(.task)
        await store.receive(\.unlocked)
        XCTAssertTrue(store.state.isLocked)
        XCTAssertNil(store.state.lockMessage)
        await store.send(.newTaskTapped)
        XCTAssertNil(store.state.quickAdd)
        await task.cancel()
    }

    func testComingBackAfterAMinuteAwayLocksAgain() async {
        let (store, time) = locked(.always())
        let task = await store.send(.task)
        await store.receive(\.unlocked)

        await store.send(.backgrounded)
        time.setValue(now + 30)
        await store.send(.foregrounded)
        XCTAssertFalse(store.state.isLocked)

        await store.send(.backgrounded)
        time.setValue(now + 120)
        await store.send(.foregrounded)
        XCTAssertTrue(store.state.isLocked)
        await store.receive(\.unlocked)
        XCTAssertFalse(store.state.isLocked)
        await task.cancel()
    }

    func testWithTheLockOffNothingAsks() async {
        // The lock's test value fails if anything asks it to confirm.
        let time = LockIsolated(now)
        let store = makeStore(.phone)
        store.dependencies.date = DateGenerator { time.value }

        let task = await store.send(.task)
        await store.send(.backgrounded)
        time.setValue(now + 3600)
        await store.send(.foregrounded)
        XCTAssertFalse(store.state.isLocked)
        await task.cancel()
    }

    func testAFailedUnlockSaysWhyAndStaysLocked() async {
        let (store, _) = locked(.always(failing: .failed("Face ID didn't recognise you")))

        let task = await store.send(.task)
        await store.receive(\.unlocked)
        XCTAssertTrue(store.state.isLocked)
        XCTAssertNotNil(store.state.lockMessage)
        XCTAssertFalse(store.state.isUnlocking)
        await task.cancel()
    }

    func testADeviceWithoutAPasscodeTurnsTheLockOffAndSaysSo() async {
        let (store, _) = locked(.always(failing: .noPasscode))

        let task = await store.send(.task)
        await store.receive(\.unlocked)
        XCTAssertFalse(store.state.isLocked)
        XCTAssertFalse(store.state.locksApp)
        XCTAssertNotNil(store.state.lockAlert)
        await task.cancel()
    }

    func testTurningTheAPIOffTellsTheSession() async {
        let session = InMemorySession()
        let store = makeStore(.mac, session: session)

        await store.send(.apiSwitched(false))
        await store.finish()

        XCTAssertFalse(store.state.isAPIEnabled)
        let enabled = await session.client.isAPIEnabled()
        XCTAssertFalse(enabled)
    }

    func testAnAPISwitchThatFailsPutsTheSwitchBackAndSaysWhy() async {
        let store = makeStore(.mac)
        store.dependencies.session.setAPIEnabled = { _ in throw SessionError.apiSwitchFailed(reason: "socket busy") }

        await store.send(.apiSwitched(false))
        await store.receive(\.apiEnabledLoaded)
        await store.receive(\.apiSwitchFailed)

        XCTAssertTrue(store.state.isAPIEnabled)
        XCTAssertNotNil(store.state.apiSwitchError)
    }

    func testReportABugOpensANewReport() async {
        let store = makeStore(.mac)
        store.dependencies.models.selection = { _ in "gemma-4-e2b" }

        await store.send(.reportBugTapped)

        XCTAssertEqual(store.state.bugReport?.report.device.assistantModel, "gemma-4-e2b")
    }

    func testReportABugWhileRecordingStepsStopsTheRecording() async {
        let store = makeStore(.mac)
        store.dependencies.models.selection = { _ in "gemma-4-e2b" }

        await store.send(.recordStepsTapped)
        XCTAssertTrue(store.state.isRecordingSteps)
        await store.send(.reportBugTapped)
        await store.receive(\.stopRecordingTapped)

        XCTAssertFalse(store.state.isRecordingSteps)
    }
}

/// A store client whose saves are made as soon as they're asked for, in
/// that order, but whose replies wait until the test releases them, by the
/// save's number.
private final class HeldReplies: Sendable {
    /// The number of each save whose reply is waiting, as it starts to wait.
    let held: AsyncStream<Int>
    private let hold: AsyncStream<Int>.Continuation
    private let waiting = LockIsolated<[Int: CheckedContinuation<Void, Never>]>([:])

    init() {
        (held, hold) = AsyncStream.makeStream()
    }

    func client(_ inner: StoreClient) -> StoreClient {
        var client = inner
        client.save = { [waiting, hold] operation in
            let saved = try await inner.save(operation)
            await withCheckedContinuation { continuation in
                waiting.withValue { $0[saved.number] = continuation }
                hold.yield(saved.number)
            }
            return saved
        }
        return client
    }

    /// Lets the reply to save `number` through.
    func release(_ number: Int) {
        let continuation = waiting.withValue { $0.removeValue(forKey: number) }
        continuation?.resume()
    }
}
