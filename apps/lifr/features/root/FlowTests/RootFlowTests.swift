import ComposableArchitecture
import Foundation
import LifrAPI
import LifrCore
import LifrGlance
import LifrGlanceClient
import LifrLockClient
import LifrRemindersClient
import LifrRoot
import LifrSessionClient
import LifrShortcuts
import LifrStoreClient
import LifrSyncClient
import LifrSyncReview
import LifrTaskDetail
import XCTest

/// The spec's eight UX flows, each through the whole app as the person would
/// go: the root store, its screens and the in-memory store and session.
@MainActor
final class RootFlowTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")

    private func app(
        _ platform: RootFeature.Platform, _ client: StoreClient, session: InMemorySession = InMemorySession(),
        sync: SyncClient = .off, glance: GlanceClient = .testValue, lock: LockClient = .testValue
    ) async -> (TestStoreOf<RootFeature>, TestStoreTask) {
        let store = TestStore(initialState: RootFeature.State(platform: platform)) {
            RootFeature()
        } withDependencies: { [now] in
            // A minute passes between changes, as it would for a person.
            let clock = LockIsolated(now)
            $0.store = client
            $0.session = session.client
            $0.reminders = RemindersClient.inMemory().client
            $0.sync = sync
            $0.glance = glance
            $0.lock = lock
            $0.continuousClock = TestClock()
            $0.date = DateGenerator {
                clock.withValue {
                    $0 += 60
                    return $0
                }
            }
            $0.timeZone = .gmt
            // Random, so records made on two devices never share an ID.
            $0.uuid = UUIDGenerator { UUID() }
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
        }
        store.exhaustivity = .off
        let task = await store.send(.task)
        await store.receive(\.loaded)
        return (store, task)
    }

    private func statusChanges(of id: UUID, in workspace: Workspace?) -> [HistoryEvent] {
        workspace?.history.filter { $0.taskID == id && $0.field == .status } ?? []
    }

    /// 1. Capture on iPhone, finish on Mac.
    func testCaptureOnIPhoneFinishOnMac() async throws {
        let marathon = records.project("Half marathon", in: health)
        let (phoneClient, _) = StoreClient.inMemorySyncing(
            records.workspace, actor: .user(device: "iPhone", surface: .iPhone))
        let (macClient, macSync) = StoreClient.inMemorySyncing(
            records.workspace, actor: .user(device: "Mac", surface: .mac))
        let (phone, phoneTask) = await app(.phone, phoneClient)
        let (mac, macTask) = await app(.mac, macClient)

        await phone.send(.newTaskTapped)
        await phone.send(.quickAdd(.presented(.binding(.set(\.title, "Buy running shoes")))))
        await phone.send(.quickAdd(.presented(.binding(.set(\.avenueID, health.id)))))
        await phone.send(.quickAdd(.presented(.binding(.set(\.projectID, marathon.id)))))
        await phone.send(.quickAdd(.presented(.binding(.set(\.due, .tomorrow)))))
        await phone.send(.quickAdd(.presented(.binding(.set(\.priority, .high)))))
        await phone.send(.quickAdd(.presented(.addTapped)))
        await phone.receive(\.quickAdd.presented.delegate.saved)
        XCTAssertEqual(phone.state.upcoming.list.rows.map(\.title), ["Buy running shoes"])
        let shoes = try XCTUnwrap(phone.state.upcoming.list.rows.first?.id)

        // Both devices come online.
        let synced = try XCTUnwrap(phone.state.workspace)
        macSync(now: now) { $0.applySynced(tasks: Array(synced.tasks.values), history: synced.history) }
        await mac.receive(\.updated)
        await mac.send(.selected(.upcoming))
        XCTAssertEqual(mac.state.upcoming.list.rows.map(\.title), ["Buy running shoes"])

        await mac.send(.upcoming(.list(.rowTapped(shoes))))
        await mac.send(.newSubtaskTapped)
        for title in ["Get a gait analysis", "Try on 3 pairs", "Return extras"] {
            await mac.send(.quickAdd(.presented(.binding(.set(\.title, title)))))
            await mac.send(.quickAdd(.presented(.addTapped)))
            await mac.receive(\.quickAdd.presented.delegate.saved)
        }
        await mac.send(.statusShortcut(.inProgress))
        await mac.receive(\.upcoming.list.delegate.saved)

        let workspace = try XCTUnwrap(mac.state.workspace)
        XCTAssertEqual(workspace.tasks[shoes]?.status, .inProgress)
        XCTAssertEqual(
            workspace.subtasks(of: shoes).map(\.title).sorted(),
            [
                "Get a gait analysis", "Return extras", "Try on 3 pairs",
            ])
        let history = workspace.history.filter { $0.taskID == shoes }
        XCTAssertEqual(Set(history.map(\.actor.device)), ["iPhone", "Mac"])
        XCTAssertEqual(history.filter { $0.kind == .subtaskAdded }.count, 3)
        await phoneTask.cancel()
        await macTask.cancel()
    }

    /// 2. Try to finish while Sub tasks are open.
    func testFinishingWithOpenSubtasksAsksThenDropsThem() async throws {
        let shoes = records.task("Buy running shoes", in: health, status: .inProgress, due: 0)
        records.task("Get a gait analysis", in: health, parent: shoes, status: .done)
        let pairs = records.task("Try on 3 pairs", in: health, parent: shoes)
        let extras = records.task("Return extras", in: health, parent: shoes)
        let (mac, task) = await app(.mac, .inMemory(records.workspace))
        let before = statusChanges(of: shoes.id, in: mac.state.workspace).count

        await mac.send(.today(.list(.rowTapped(shoes.id))))
        await mac.send(.statusShortcut(.done))
        await mac.receive(\.today.list.detail.presented.saved)
        XCTAssertNotNil(mac.state.today.list.detail?.prompt)
        XCTAssertEqual(mac.state.workspace?.tasks[shoes.id]?.status, .inProgress)

        await mac.send(.today(.list(.detail(.presented(.prompt(.presented(.finishDroppingSubtasks(task: shoes.id))))))))
        await mac.receive(\.today.delegate.saved)

        let workspace = try XCTUnwrap(mac.state.workspace)
        XCTAssertEqual(workspace.tasks[shoes.id]?.status, .done)
        XCTAssertEqual(workspace.tasks[pairs.id]?.status, .wontDo)
        XCTAssertEqual(workspace.tasks[extras.id]?.status, .wontDo)
        let written =
            statusChanges(of: shoes.id, in: workspace).count - before
            + statusChanges(of: pairs.id, in: workspace).count + statusChanges(of: extras.id, in: workspace).count
        XCTAssertEqual(written, 3)
        await task.cancel()
    }

    /// 3. Drop a task that has open Sub tasks.
    func testDroppingWithOpenSubtasksAsksFirst() async throws {
        let plan = records.task("Plan long runs", in: health, status: .inProgress, due: 0)
        let week = records.task("Week 1", in: health, parent: plan)
        let (mac, task) = await app(.mac, .inMemory(records.workspace))

        await mac.send(.today(.list(.rowAction(plan.id, .status(.wontDo)))))
        await mac.receive(\.today.list.saved)
        XCTAssertNotNil(mac.state.today.list.prompt)
        XCTAssertEqual(mac.state.workspace?.tasks[plan.id]?.status, .inProgress)

        await mac.send(.today(.list(.prompt(.presented(.dropWithSubtasks(task: plan.id))))))
        await mac.receive(\.today.delegate.saved)

        XCTAssertEqual(mac.state.workspace?.tasks[plan.id]?.status, .wontDo)
        XCTAssertEqual(mac.state.workspace?.tasks[week.id]?.status, .wontDo)
        await task.cancel()
    }

    /// 4. Weekly Procrastinated review.
    func testProcrastinatedReview() async throws {
        let plan = records.task("Plan long runs", in: health, status: .inProgress, idle: 12)
        let physio = records.task("Book physio", in: health, status: .procrastinated, due: -3)
        let stretch = records.task("Stretch plan", in: health, status: .procrastinated)
        let (mac, task) = await app(.mac, .inMemory(records.workspace))

        // From the sidebar, or the Today screen.
        XCTAssertEqual(mac.state.today.procrastinatedCount, 3)
        await mac.send(.selected(.procrastinated))
        XCTAssertEqual(Set(mac.state.procrastinated.list.rows.map(\.id)), [plan.id, physio.id, stretch.id])
        XCTAssertEqual(mac.state.procrastinated.list.sections.flatMap(\.notes).count, 3)

        // Restart counts as activity, so the row leaves the list.
        await mac.send(.procrastinated(.list(.rowAction(plan.id, .status(.inProgress)))))
        await mac.receive(\.procrastinated.list.delegate.saved)
        XCTAssertFalse(mac.state.procrastinated.list.rows.contains { $0.id == plan.id })

        // A task set by hand stays when rescheduled, until its status changes.
        await mac.send(.procrastinated(.list(.rowAction(physio.id, .reschedule))))
        let nextWeek = CalendarDate(now + 7 * 24 * 60 * 60, in: .gmt)
        await mac.send(.procrastinated(.list(.reschedule(.presented(.delegate(.picked(physio.id, nextWeek)))))))
        await mac.receive(\.procrastinated.list.delegate.saved)
        XCTAssertTrue(mac.state.procrastinated.list.rows.contains { $0.id == physio.id })

        // Won't Do closes it.
        await mac.send(.procrastinated(.list(.rowAction(stretch.id, .status(.wontDo)))))
        await mac.receive(\.procrastinated.list.delegate.saved)
        XCTAssertFalse(mac.state.procrastinated.list.rows.contains { $0.id == stretch.id })
        XCTAssertEqual(mac.state.today.procrastinatedCount, 1)
        await task.cancel()
    }

    /// 5. Reopen a finished task by adding a Sub task.
    func testAddingASubtaskReopensADoneTaskWithUndo() async throws {
        let shoes = records.task("Buy running shoes", in: health, due: 0)
        records.setStatus(.done, of: shoes)
        let (mac, task) = await app(.mac, .inMemory(records.workspace))

        await mac.send(.today(.list(.rowTapped(shoes.id))))
        await mac.send(.today(.list(.detail(.presented(.binding(.set(\.newSubtaskTitle, "Buy socks")))))))
        await mac.send(.today(.list(.detail(.presented(.addSubtaskSubmitted)))))
        await mac.receive(\.today.delegate.saved)
        XCTAssertEqual(mac.state.workspace?.tasks[shoes.id]?.status, .inProgress)
        XCTAssertNotNil(mac.state.today.list.detail?.banner)

        await mac.send(.today(.list(.detail(.presented(.bannerUndoTapped)))))
        await mac.receive(\.today.delegate.saved)
        let workspace = try XCTUnwrap(mac.state.workspace)
        XCTAssertEqual(workspace.tasks[shoes.id]?.status, .done)
        XCTAssertTrue(workspace.subtasks(of: shoes.id).isEmpty)
        await task.cancel()
    }

    /// 6. Connect Claude Code: the approval sheet, then the dot.
    func testConnectingClaudeCode() async throws {
        let session = InMemorySession()
        let (mac, task) = await app(.mac, .inMemory(records.workspace), session: session)
        let request = ApprovalRequest(
            id: UUID(), identity: "com.anthropic.claude-code", client: "Claude Code", method: .searchTasks,
            requestedAt: now)

        session.request(request)
        await mac.receive(\.sessionEvent)
        XCTAssertEqual(mac.state.approval?.request.client, "Claude Code")

        await mac.send(.approval(.presented(.approveTapped)))
        await mac.receive(\.approval.presented.answered)
        await mac.receive(\.approval.dismiss)
        XCTAssertNil(mac.state.approval)
        XCTAssertEqual(session.apps.first?.grant.scopes, [.read])
        XCTAssertTrue(mac.state.hasConnectedApp)

        // Settings lists it with its last use.
        session.use("com.anthropic.claude-code", at: now)
        await mac.receive(\.sessionEvent)
        XCTAssertEqual(mac.state.apps.first?.lastUsedAt, now)
        await task.cancel()
    }

    /// 7. Use the CLI from a terminal: its changes go through the app and
    /// show on every screen as they happen.
    func testChangesFromTheCLIShowRightAway() async throws {
        let run = records.task("Long run", in: health, due: 0)
        let (client, cli) = StoreClient.inMemorySyncing(records.workspace)
        let (mac, task) = await app(.mac, client)

        cli(now: now) { workspace in
            _ = try? workspace.setStatus(
                .inProgress, of: run.id, in: Context(actor: .agent("lifr", device: "Mac", surface: .cli), now: now))
        }
        await mac.receive(\.updated)

        XCTAssertEqual(mac.state.today.list.rows.first?.status, .inProgress)
        XCTAssertEqual(statusChanges(of: run.id, in: mac.state.workspace).last?.actor.kind, .agent)
        XCTAssertNil(mac.state.syncBanner)
        await task.cancel()
    }

    /// Shortcuts or Siri adds a task and finishes another while the app is
    /// open: each shows on Today as it happens, made by Shortcuts.
    func testChangesFromShortcutsShowRightAway() async throws {
        let run = records.task("Long run", in: health, due: 0)
        let (client, sync) = StoreClient.inMemorySyncing(records.workspace)
        let (phone, task) = await app(.phone, client)
        let now = now
        let shortcuts = Shortcuts(
            store: sync.writer(as: .user(device: "iPhone", surface: .shortcuts)), locksApp: { false },
            now: { now }, timeZone: .gmt)

        let added = try await withDependencies {
            $0.date.now = now
            $0.uuid = UUIDGenerator { UUID() }
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
        } operation: {
            let added = try await shortcuts.add("Buy oat milk", avenue: nil, due: .today)
            try await shortcuts.finish(run.id)
            return added
        }
        await phone.receive(\.updated)
        await phone.receive(\.updated)

        let titles = phone.state.today.list.rows.map(\.title)
        XCTAssertTrue(titles.contains("Buy oat milk"))
        XCTAssertEqual(phone.state.workspace?.tasks[run.id]?.status, .done)
        XCTAssertEqual(phone.state.workspace?.journal.last?.actor.surface, .shortcuts)
        XCTAssertEqual(phone.state.workspace?.tasks[added.id]?.title, "Buy oat milk")
        await task.cancel()
    }

    /// The widgets get Today after each change, from the app or from
    /// Shortcuts, and only how many while the lock is on.
    func testTheWidgetsFollowTodayAndOnlyCountWhileLocked() async throws {
        let run = records.task("Long run", in: health, due: 0)
        let stretch = records.task("Stretch", in: health, due: -1)
        _ = records.task("Book a physio", in: health, due: 3)
        let (client, sync) = StoreClient.inMemorySyncing(records.workspace)
        let (glance, written) = GlanceClient.inMemory()
        let (phone, task) = await app(.phone, client, glance: glance, lock: .always())
        await phone.receive(\.remindersScheduled)

        XCTAssertEqual(written.value?.count, 2)
        XCTAssertEqual(written.value?.overdue, 1)
        XCTAssertEqual(Set(written.value?.rows.map(\.title) ?? []), ["Long run", "Stretch"])
        XCTAssertEqual(written.value?.rows.first { $0.id == stretch.id }?.isOverdue, true)

        let now = now
        let shortcuts = Shortcuts(
            store: sync.writer(as: .user(device: "iPhone", surface: .shortcuts)), locksApp: { false },
            now: { now }, timeZone: .gmt)
        try await withDependencies {
            $0.date.now = now
            $0.uuid = UUIDGenerator { UUID() }
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
        } operation: {
            try await shortcuts.finish(run.id)
        }
        await phone.receive(\.updated)
        await phone.receive(\.remindersScheduled)
        XCTAssertEqual(written.value?.rows.map(\.title), ["Stretch"])

        await phone.send(.settings(.lockToggled(true)))
        await phone.receive(\.settings.lockConfirmed)
        XCTAssertEqual(written.value?.isLocked, true)
        XCTAssertEqual(written.value?.count, 1)
        XCTAssertEqual(written.value?.rows, [])

        await phone.send(.settings(.lockToggled(false)))
        await phone.receive(\.settings.lockConfirmed)
        XCTAssertEqual(written.value?.isLocked, false)
        XCTAssertEqual(written.value?.rows.map(\.title), ["Stretch"])
        await task.cancel()
    }

    /// 8. Offline edits on two devices.
    func testOfflineEditsOnTwoDevicesReopenTheParent() async throws {
        let shoes = records.task("Buy running shoes", in: health, status: .inProgress, due: 0)
        let pairs = records.task("Try on 3 pairs", in: health, parent: shoes)
        var mac = records
        let socks = mac.task("Buy socks", in: health, parent: shoes)
        records.setStatus(.done, of: pairs)
        records.setStatus(.done, of: shoes)
        let (client, sync) = StoreClient.inMemorySyncing(
            records.workspace, actor: .user(device: "iPhone", surface: .iPhone))
        let (phone, task) = await app(.phone, client)
        XCTAssertEqual(phone.state.workspace?.tasks[shoes.id]?.status, .done)

        let socksOnMac = try XCTUnwrap(mac.workspace.tasks[socks.id])
        sync(now: now) { $0.applySynced(tasks: [socksOnMac]) }
        await phone.receive(\.updated)

        let workspace = try XCTUnwrap(phone.state.workspace)
        XCTAssertEqual(workspace.tasks[shoes.id]?.status, .inProgress)
        XCTAssertNotNil(phone.state.syncBanner)
        let reopened = statusChanges(of: shoes.id, in: workspace).last
        XCTAssertEqual(reopened?.actor.kind, .system)
        XCTAssertEqual(reopened?.reason, .reconciledClosedParent)
        await task.cancel()
    }

    /// 9. Editing on two devices: what merges merges, what can't waits for
    /// review, and the choice reaches the other device.
    func testEditsOnTwoDevicesMergeOrWaitForReview() async throws {
        let run = records.task("Run", in: health, due: 0)
        let socks = records.task("Buy socks", in: health)
        let cloud = InMemoryCloud()
        func device(_ name: String, _ surface: Actor.Surface) -> (StoreClient, SyncClient) {
            let client = StoreClient.inMemory(records.workspace, actor: .user(device: name, surface: surface))
            let syncer = Syncer(cloud: cloud.connection(device: name), store: client, storage: .inMemory())
            // Syncs only when asked, so each step syncs where the test says.
            return (client, .syncing(syncer, settle: .seconds(60 * 60)))
        }
        let (phoneClient, phoneSync) = device("iPhone", .iPhone)
        let (macClient, macSync) = device("Mac", .mac)
        let (phone, phoneTask) = await app(.phone, phoneClient, sync: phoneSync)
        let (mac, macTask) = await app(.mac, macClient, sync: macSync)
        await synced(phone)
        await synced(mac)

        // Different fields of one task merge without asking.
        await phone.send(.today(.list(.toggled(run.id))))
        await phone.receive(\.today.delegate.saved)
        try await change(macClient) { (workspace, context) throws(LifrError) in
            try workspace.edit(run.id, Workspace.TaskEdit(priority: .high), in: context)
        }
        await synced(phone)
        await synced(mac)
        await synced(phone)
        XCTAssertEqual(phone.state.workspace?.tasks[run.id]?.priority, .high)
        XCTAssertEqual(mac.state.workspace?.tasks[run.id]?.status, .done)
        XCTAssertEqual(mac.state.sync.review, [])

        // The same field changed on both waits for review; Later keeps it.
        try await change(phoneClient) { (workspace, context) throws(LifrError) in
            try workspace.edit(socks.id, Workspace.TaskEdit(title: "Buy wool socks"), in: context)
        }
        try await change(macClient) { (workspace, context) throws(LifrError) in
            try workspace.edit(socks.id, Workspace.TaskEdit(title: "Buy running socks"), in: context)
        }
        await synced(phone)
        await synced(mac)
        XCTAssertEqual(mac.state.sync.review.count, 1)
        await mac.send(.syncReviewTapped)
        XCTAssertEqual(mac.state.syncReview?.rows.first?.device, "iPhone")
        await mac.send(.syncReview(.presented(.laterTapped)))
        await mac.receive(\.syncReview.presented.delegate.finished)
        XCTAssertNil(mac.state.syncReview)
        await synced(mac)
        XCTAssertEqual(mac.state.sync.review.count, 1)

        // Choosing the iPhone's title sends it everywhere, as one change.
        await mac.send(.syncReviewTapped)
        let item = try XCTUnwrap(mac.state.syncReview?.items.first)
        await mac.send(.syncReview(.presented(.chose(item.id, .title, .theirs))))
        await mac.send(.syncReview(.presented(.doneTapped)))
        await mac.receive(\.syncReview.presented.delegate.finished)
        await synced(mac)
        await synced(phone)
        XCTAssertEqual(mac.state.sync.review, [])
        XCTAssertEqual(mac.state.workspace?.tasks[socks.id]?.title, "Buy wool socks")
        XCTAssertEqual(phone.state.workspace?.tasks[socks.id]?.title, "Buy wool socks")
        XCTAssertEqual(phone.state.sync.review, [])

        // Trashed on the iPhone while the Mac edited it: keeping it restores it.
        try await change(phoneClient) { (workspace, context) throws(LifrError) in
            try workspace.moveToTrash(socks.id, in: context)
        }
        try await change(macClient) { (workspace, context) throws(LifrError) in
            try workspace.edit(socks.id, Workspace.TaskEdit(priority: .low), in: context)
        }
        await synced(phone)
        await synced(mac)
        let removed = try XCTUnwrap(mac.state.sync.review.first)
        XCTAssertEqual(removed.conflict.kind, .removed(by: .theirs))
        await mac.send(.syncReviewTapped)
        await mac.send(.syncReview(.presented(.doneTapped)))
        await mac.receive(\.syncReview.presented.delegate.finished)
        await synced(mac)
        await synced(phone)
        XCTAssertNil(phone.state.workspace?.tasks[socks.id]?.deletedAt)
        XCTAssertEqual(phone.state.workspace?.tasks[socks.id]?.priority, .low)

        await phoneTask.cancel()
        await macTask.cancel()
    }

    /// Sync Now, then waits for the sync to finish.
    private func synced(_ store: TestStoreOf<RootFeature>) async {
        await store.send(.syncNowTapped)
        for _ in 0..<6 {
            await store.receive(\.syncChanged)
            if store.state.sync.status != .syncing, store.state.sync.lastSynced != nil { break }
        }
    }

    /// Changes the records on a device directly, as another screen would.
    private func change(_ client: StoreClient, _ operation: @escaping StoreOperation) async throws {
        _ = try await withDependencies { [now] in
            $0.date.now = now
            $0.uuid = UUIDGenerator { UUID() }
        } operation: {
            try await client.save(operation)
        }
    }
}
