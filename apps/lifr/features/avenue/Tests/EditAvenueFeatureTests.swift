import ComposableArchitecture
import Foundation
import LifrAvenue
import LifrCore
import LifrStoreClient
import XCTest

@MainActor
final class EditAvenueFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")
    private lazy var home = records.avenue("HOME", "Home")

    private func dependencies(_ values: inout DependencyValues, store: StoreClient) {
        values.store = store
        values.date.now = now
        values.timeZone = .gmt
        values.uuid = .incrementing
        values.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
        values.dismiss = DismissEffect {}
    }

    private func edit(_ avenue: Avenue) throws -> TestStoreOf<EditAvenueFeature> {
        let client = StoreClient.inMemory(records.workspace)
        let workspace = records.workspace
        let state = try XCTUnwrap(EditAvenueFeature.State(id: avenue.id, workspace: workspace))
        let store = TestStore(initialState: state) {
            EditAvenueFeature()
        } withDependencies: {
            self.dependencies(&$0, store: client)
        }
        store.exhaustivity = .off
        return store
    }

    func testSavesTheNameColorAndSymbol() async throws {
        let store = try edit(health)

        await store.send(.binding(.set(\.name, "Fitness")))
        await store.send(.binding(.set(\.color, "red")))
        await store.send(.binding(.set(\.symbol, "dumbbell")))
        await store.send(.saveTapped)
        await store.receive(\.delegate.saved)

        let avenue = try await store.dependencies.store.load().avenues[health.id]
        XCTAssertEqual(avenue?.name, "Fitness")
        XCTAssertEqual(avenue?.color, "red")
        XCTAssertEqual(avenue?.symbol, "dumbbell")
    }

    func testArchivingWithOpenTasksAsksThenClosesThem() async throws {
        let stretch = records.task("Stretch", in: health)
        let store = try edit(health)

        await store.send(.archiveTapped)
        XCTAssertNotNil(store.state.confirmArchive)
        await store.send(.confirmArchive(.presented(.closeOpenTasks)))
        await store.receive(\.delegate.saved)

        let workspace = try await store.dependencies.store.load()
        XCTAssertNotNil(workspace.avenues[health.id]?.archivedAt)
        XCTAssertEqual(workspace.tasks[stretch.id]?.status, .wontDo)
    }

    func testArchivedAvenuesAndClosedProjectsAreListedToRestore() async throws {
        let marathon = records.project("Half marathon", in: health)
        _ = records.project("Garden", in: home)
        let store = try edit(home)
        await store.send(.archiveTapped)
        await store.receive(\.delegate.saved)
        let client = store.dependencies.store
        _ = try await withDependencies {
            self.dependencies(&$0, store: client)
        } operation: {
            try await client.setStatus(.completed, ofProject: marathon.id)
        }

        let workspace = try await store.dependencies.store.load()
        XCTAssertEqual(workspace.archivedRows.map(\.scope), [.avenue(home.id), .project(marathon.id)])
    }

    func testRestoresAnArchivedAvenue() async throws {
        let store = try edit(home)
        await store.send(.archiveTapped)
        await store.receive(\.delegate.saved)

        let client = store.dependencies.store
        let archived = try await client.load()
        let restoring = try XCTUnwrap(EditAvenueFeature.State(id: home.id, workspace: archived))
        XCTAssertTrue(restoring.isArchived)
        let restore = TestStore(initialState: restoring) {
            EditAvenueFeature()
        } withDependencies: {
            self.dependencies(&$0, store: client)
        }
        restore.exhaustivity = .off
        await restore.send(.restoreTapped)
        await restore.receive(\.delegate.saved)

        let workspace = try await store.dependencies.store.load()
        XCTAssertNil(workspace.avenues[home.id]?.archivedAt)
    }
}
