import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTrash
import XCTest

@MainActor
final class TrashFeatureTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_800_000_000)

    /// "Kept", and "Trashed" moved to the trash `daysAgo` days before `now`.
    private func workspace(trashedDaysAgo daysAgo: Int) -> (Workspace, UUID) {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        records.task("Kept", in: home)
        let trashed = records.task("Trashed", in: home)
        records.trash(trashed, daysAgo: daysAgo)
        return (records.workspace, trashed.id)
    }

    private func makeStore(_ workspace: Workspace) -> TestStoreOf<TrashFeature> {
        let store = TestStore(initialState: TrashFeature.State(workspace: workspace, now: now)) {
            TrashFeature()
        } withDependencies: { [now] in
            $0.date.now = now
            $0.store = .inMemory(workspace)
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return store
    }

    func testEachRowSaysWhenItsDeleted() {
        let (workspace, id) = workspace(trashedDaysAgo: 10)
        let state = TrashFeature.State(workspace: workspace, now: now)
        XCTAssertEqual(state.rows.map(\.id), [id])
        XCTAssertEqual(state.rows.first?.daysLeft, 20)
    }

    func testRestoringTakesATaskOutOfTheTrash() async throws {
        let (workspace, id) = workspace(trashedDaysAgo: 1)
        let store = makeStore(workspace)
        await store.send(.restoreTapped(id))
        await store.receive(\.saved.success)
        await store.receive(\.delegate.saved)
        let saved = try await store.dependencies.store.load()
        XCTAssertNil(saved.tasks[id]?.deletedAt)
        XCTAssertTrue(store.state.rows.isEmpty)
    }

    func testEmptyingAsksFirstThenDeletesForGood() async throws {
        let (workspace, id) = workspace(trashedDaysAgo: 1)
        let store = makeStore(workspace)
        await store.send(.emptyTapped)
        XCTAssertNotNil(store.state.confirmation)
        await store.send(.confirmation(.dismiss))
        XCTAssertEqual(store.state.rows.count, 1)

        await store.send(.emptyTapped)
        await store.send(.confirmation(.presented(.empty)))
        await store.receive(\.saved.success)
        await store.receive(\.delegate.saved)
        let saved = try await store.dependencies.store.load()
        XCTAssertNil(saved.tasks[id])
        XCTAssertEqual(saved.tasks.values.map(\.title), ["Kept"])
        XCTAssertTrue(store.state.rows.isEmpty)
    }

    func testAnEmptyTrashAsksNothing() async {
        let store = makeStore(Workspace())
        await store.send(.emptyTapped)
        XCTAssertNil(store.state.confirmation)
    }
}
