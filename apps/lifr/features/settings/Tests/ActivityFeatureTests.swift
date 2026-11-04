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
            [["Run 5 km", "Priority", "none", "high"], ["Run 5 km", "Title", "Run", "Run 5 km"]])
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
        XCTAssertEqual(rows.first?.summary, "Reverted a change")
        XCTAssertEqual(rows.first?.author, "This device")
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
