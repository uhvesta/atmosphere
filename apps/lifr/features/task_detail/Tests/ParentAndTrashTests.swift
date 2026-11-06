import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskDetail
import XCTest

@MainActor
final class ParentAndTrashTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")

    private func makeStore(_ task: LifrTask) throws -> TestStoreOf<TaskDetailFeature> {
        let state = try XCTUnwrap(
            TaskDetailFeature.State(id: task.id, workspace: records.workspace, now: now, timeZone: .gmt))
        let store = TestStore(initialState: state) {
            TaskDetailFeature()
        } withDependencies: {
            $0.store = .inMemory(records.workspace)
            $0.date.now = now
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
            $0.timeZone = .gmt
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return store
    }

    func testOffersOpenTopLevelTasksInTheSameProjectAsParents() throws {
        let marathon = records.project("Half marathon", in: health)
        let shoes = records.task("Buy running shoes", in: health, project: marathon)
        let plan = records.task("Training plan", in: health, project: marathon)
        records.task("Sign up", in: health, project: marathon, status: .done)
        records.task("Stretch", in: health)
        records.task("Pick a route", in: health, project: marathon, parent: plan)

        let store = try makeStore(shoes)

        XCTAssertEqual(store.state.parentChoices.map(\.name), ["Training plan"])
    }

    func testATaskWithSubtasksCannotBecomeOne() throws {
        let plan = records.task("Training plan", in: health)
        records.task("Stretch", in: health)
        records.task("Pick a route", in: health, parent: plan)

        let store = try makeStore(plan)

        XCTAssertEqual(store.state.parentChoices, [])
    }

    func testMakesASubtaskAndTurnsItBack() async throws {
        let plan = records.task("Training plan", in: health)
        let stretch = records.task("Stretch", in: health)
        let store = try makeStore(stretch)

        await store.send(.parentPicked(plan.id))
        await store.receive(\.delegate.saved)
        XCTAssertTrue(store.state.isSubtask)

        await store.send(.parentPicked(nil))
        await store.receive(\.delegate.saved)
        XCTAssertFalse(store.state.isSubtask)
    }

    func testMovesTheTaskAndItsSubtasksToTheTrash() async throws {
        let plan = records.task("Training plan", in: health)
        let route = records.task("Pick a route", in: health, parent: plan)
        let store = try makeStore(plan)

        await store.send(.trashTapped)
        await store.receive(\.delegate.saved)

        let workspace = try await store.dependencies.store.load()
        XCTAssertNotNil(workspace.tasks[plan.id]?.deletedAt)
        XCTAssertNotNil(workspace.tasks[route.id]?.deletedAt)
    }
}
