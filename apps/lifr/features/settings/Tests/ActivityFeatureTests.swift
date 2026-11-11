import ComposableArchitecture
import Foundation
import LifrCore
import LifrSettings
import LifrStoreClient
import XCTest

@MainActor
final class ActivityFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    /// A task "Run", edited from the Mac and then over MCP by Claude Code.
    private func activity() async throws -> (TestStoreOf<ActivityFeature>, LifrTask) {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let health = records.avenue("HEALTH", "Health")
        let task = records.task("Run", in: health)
        let mac = StoreClient.inMemory(records.workspace, actor: .user(device: "Mac", surface: .mac))
        let workspace = try await withDependencies { [now] in
            $0.date.now = now
            $0.uuid = .incrementing
        } operation: {
            _ = try await mac.edit(task.id, .init(title: "Run 5 km", priority: .high))
            let mcp = StoreClient.inMemory(
                try await mac.load(), actor: .agent("Claude Code", device: "Mac", surface: .mcp))
            return try await mcp.edit(task.id, .init(title: "Run 10 km")).workspace
        }
        let store = TestStore(initialState: ActivityFeature.State(workspace: workspace)) {
            ActivityFeature()
        } withDependencies: { [now] in
            $0.store = .inMemory(workspace)
            $0.date.now = now
            $0.uuid = UUIDGenerator { UUID() }
        }
        store.exhaustivity = .off
        return (store, task)
    }

    func testShowsEachChangeNewestFirstWithWhoMadeItAndWhatChanged() async throws {
        let (store, _) = try await activity()
        let rows = store.state.rows(now: now, calendar: .current)

        XCTAssertEqual(rows.map(\.summary), ["Changed task “Run 10 km”", "Changed task “Run 5 km”"])
        XCTAssertEqual(rows.map(\.author), ["Claude Code · MCP", "Mac"])
        XCTAssertEqual(
            rows[1].changes.map { [$0.record, $0.field, $0.before, $0.after] },
            [["Run 5 km", "Title", "Run", "Run 5 km"], ["Run 5 km", "Priority", "None", "High"]])
    }

    func testChangesOlderThanTheUndoWindowShowOnceAskedForAndCantBeReverted() async throws {
        let (activity, _) = try await activity()
        let client = StoreClient.inMemory(activity.state.workspace)
        try await client.archiveJournal(now.addingTimeInterval(1))
        let workspace = try await client.load()
        let store = TestStore(initialState: ActivityFeature.State(workspace: workspace)) {
            ActivityFeature()
        } withDependencies: {
            $0.store = client
        }
        store.exhaustivity = .off
        XCTAssertEqual(store.state.rows(now: now, calendar: .current), [])

        await store.send(.olderTapped)
        await store.receive(\.olderLoaded)

        let rows = store.state.rows(now: now, calendar: .current)
        XCTAssertEqual(rows.map(\.summary), ["Changed task “Run 10 km”", "Changed task “Run 5 km”"])
        XCTAssertEqual(rows.map(\.author), ["Claude Code · MCP", "Mac"])
        XCTAssertEqual(
            rows[1].changes.map { [$0.record, $0.field, $0.before, $0.after] },
            [["Run 5 km", "Title", "Run", "Run 5 km"], ["Run 5 km", "Priority", "None", "High"]])
        XCTAssertEqual(rows.map(\.canRevert), [false, false])
    }

    func testFiltersByAuthorRecordAndDate() async throws {
        let (store, _) = try await activity()

        await store.send(.binding(.set(\.author, .surface(.mcp))))
        XCTAssertEqual(store.state.rows(now: now, calendar: .current).map(\.author), ["Claude Code · MCP"])

        await store.send(.binding(.set(\.author, nil)))
        await store.send(.binding(.set(\.kind, .label)))
        XCTAssertEqual(store.state.rows(now: now, calendar: .current), [])

        await store.send(.binding(.set(\.kind, nil)))
        await store.send(.binding(.set(\.period, .today)))
        XCTAssertEqual(store.state.rows(now: now.addingTimeInterval(2 * 24 * 60 * 60), calendar: .current), [])
    }

    func testTrashingATaskSaysItsInTheTrashNotWhen() async throws {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let task = records.task("Run", in: records.avenue("HEALTH", "Health"))
        let client = StoreClient.inMemory(records.workspace, actor: .user(device: "Mac", surface: .mac))
        let workspace = try await withDependencies { [now] in
            $0.date.now = now
            $0.uuid = .incrementing
        } operation: {
            try await client.moveToTrash(task.id).snapshot.workspace
        }
        let rows = ActivityFeature.State(workspace: workspace).rows(now: now, calendar: .current)

        XCTAssertEqual(rows.first?.summary, "Trashed task “Run”")
        XCTAssertEqual(
            rows.first?.changes.map { [$0.field, $0.before, $0.after] }, [["In Trash", "No", "Yes"]])
    }

    func testAddingATaskListsWhatItWasSetToButNotItsNameOrCode() async throws {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let health = records.avenue("HEALTH", "Health")
        let client = StoreClient.inMemory(records.workspace, actor: .user(device: "Mac", surface: .mac))
        let workspace = try await withDependencies { [now] in
            $0.date.now = now
            $0.uuid = .incrementing
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
        } operation: {
            try await client.createTask(.init(title: "Run", avenueID: health.id)).snapshot.workspace
        }
        let rows = ActivityFeature.State(workspace: workspace).rows(now: now, calendar: .current)
        let fields = rows.first?.changes.map(\.field) ?? []

        XCTAssertEqual(rows.first?.summary, "Added task “Run”")
        XCTAssertTrue(fields.contains("Avenue"))
        XCTAssertFalse(fields.contains("Title"))
        XCTAssertFalse(fields.contains("Code"))
    }

    func testRevertingAProjectWithItsTaskStillNamesTheProject() async throws {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let health = records.avenue("HEALTH", "Health")
        let client = StoreClient.inMemory(records.workspace, actor: .user(device: "Mac", surface: .mac))
        let workspace = try await withDependencies { [now] in
            $0.date.now = now
            $0.uuid = .incrementing
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
        } operation: {
            let created = try await client.createProject("Trip", in: health.id).snapshot.workspace
            let project = try XCTUnwrap(created.projects.values.first)
            _ = try await client.createTask(.init(title: "Pack", avenueID: health.id, projectID: project.id))
            let entries = try await client.load().journal.map(\.id)
            return try await client.revert(entries: entries).snapshot.workspace
        }
        let rows = ActivityFeature.State(workspace: workspace).rows(now: now, calendar: .current)
        let reverted = try XCTUnwrap(rows.first)
        let values = reverted.changes.flatMap { [$0.before, $0.after] }.compactMap { $0 }

        XCTAssertEqual(reverted.summary, "Reverted 2 changes")
        XCTAssertTrue(values.contains("Trip"))
        XCTAssertFalse(values.contains("something removed"))
    }

    func testRevertingAChangePutsItBackAndMarksIt() async throws {
        let (store, task) = try await activity()
        let newest = try XCTUnwrap(store.state.workspace.journal.last)

        await store.send(.revertTapped(newest.id))
        XCTAssertNotNil(store.state.confirm)
        await store.send(.confirm(.presented(.revert([newest.id]))))
        await store.receive(\.saved.success)
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.workspace.tasks[task.id]?.title, "Run 5 km")
        XCTAssertNil(store.state.alert)
        let rows = store.state.rows(now: now, calendar: .current)
        XCTAssertEqual(rows.first?.summary, "Reverted a change to “Run 5 km”")
        XCTAssertEqual(rows.first?.author, "iPhone")
        XCTAssertEqual(rows.dropFirst().first?.isReverted, true)
    }

    func testRevertingAnOlderChangeKeepsFieldsChangedSinceAndSaysSo() async throws {
        let (store, task) = try await activity()
        let oldest = try XCTUnwrap(store.state.workspace.journal.first)

        await store.send(.revertTapped(oldest.id))
        await store.send(.confirm(.presented(.revert([oldest.id]))))
        await store.receive(\.saved.success)

        XCTAssertEqual(store.state.workspace.tasks[task.id]?.title, "Run 10 km")
        XCTAssertEqual(store.state.workspace.tasks[task.id]?.priority, Priority.none)
        XCTAssertNotNil(store.state.alert)
    }

    func testRevertingEverythingSinceAChangeRevertsItAndTheLaterOnes() async throws {
        let (store, task) = try await activity()
        let oldest = try XCTUnwrap(store.state.workspace.journal.first)

        await store.send(.revertSinceTapped(oldest.id))
        XCTAssertNotNil(store.state.confirm)
        let ids = store.state.workspace.entries(since: oldest.at).map(\.id)
        await store.send(.confirm(.presented(.revert(ids))))
        await store.receive(\.saved.success)

        XCTAssertEqual(store.state.workspace.tasks[task.id]?.title, "Run")
        XCTAssertEqual(store.state.workspace.tasks[task.id]?.priority, Priority.none)
        XCTAssertNil(store.state.alert)
    }
}
