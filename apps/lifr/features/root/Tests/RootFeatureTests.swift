import ComposableArchitecture
import Foundation
import LifrAPI
import LifrAvenue
import LifrCore
import LifrQuickAdd
import LifrRemindersClient
import LifrRoot
import LifrSessionClient
import LifrStoreClient
import XCTest

@MainActor
final class RootFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")

    private let request = ApprovalRequest(
        id: UUID(0), identity: "com.anthropic.claude-code", client: "Claude Code", method: .searchTasks,
        requestedAt: Date(timeIntervalSince1970: 1_793_700_000))

    private func makeStore(
        _ platform: RootFeature.Platform,
        store client: StoreClient? = nil,
        session: InMemorySession = InMemorySession(),
        reminders: InMemoryReminders = RemindersClient.inMemory()
    ) -> TestStoreOf<RootFeature> {
        let client = client ?? .inMemory(records.workspace)
        let store = TestStore(initialState: RootFeature.State(platform: platform)) {
            RootFeature()
        } withDependencies: { [now] in
            $0.store = client
            if platform == .mac { $0.session = session.client }
            $0.reminders = reminders.client
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
        XCTAssertEqual(store.state.upcoming.list.rows.map(\.title), ["Stretch"])
        XCTAssertEqual(store.state.avenues.avenues.map(\.name), ["Health"])
        XCTAssertEqual(store.state.settings.avenues.map(\.name), ["Health"])
        await store.receive(\.remindersScheduled)
        XCTAssertEqual(reminders.scheduled.map(\.taskID), [run.id])
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

        await store.send(.today(.list(.toggled(run.id))))
        await store.receive(\.today.delegate.saved)

        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.status, .done)
        XCTAssertEqual(store.state.avenues.avenues.first?.openCount, 0)
        XCTAssertEqual(store.state.search.list.workspace?.tasks[run.id]?.status, .done)
        await task.cancel()
    }

    func testASyncThatFixedSomethingShowsTheBannerUntilDismissed() async {
        let parent = records.task("Buy running shoes", in: health)
        // Offline, one device adds a Sub task while the other closes the parent.
        var mac = records
        let socks = mac.task("Buy socks", in: health, parent: parent)
        records.setStatus(.done, of: parent)
        let (client, sync) = StoreClient.inMemorySyncing(records.workspace)
        let store = makeStore(.phone, store: client)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        sync(now: now) { $0.applySynced(tasks: [mac.workspace.tasks[socks.id]!]) }
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
        second.id = UUID(1)
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

    func testNewAvenueFromTheSidebarAsksForOne() async {
        let store = makeStore(.mac)
        let task = await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.newAvenueTapped)
        await store.receive(\.avenues.newAvenueTapped)
        XCTAssertNotNil(store.state.avenues.newAvenue)
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
        await store.send(.updated(Update(workspace: synced.workspace)))

        await store.send(.undoTapped(id))
        XCTAssertNotNil(store.state.undoAlert)
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.status, .done)
        XCTAssertEqual(store.state.workspace?.tasks[run.id]?.title, "Run 5 km")
        await task.cancel()
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
}

extension UUID {
    fileprivate init(_ value: Int) {
        self.init(uuidString: String(format: "00000000-0000-0000-0000-%012d", value))!
    }
}
