import ComposableArchitecture
import Foundation
import LifrAvenue
import LifrBoard
import LifrCore
import LifrStoreClient
import LifrTaskList
import XCTest

@MainActor
final class BoardFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")

    private func board(_ scope: AvenueScope) -> TestStoreOf<BoardFeature> {
        let store = TestStore(
            initialState: BoardFeature.State(
                scope: scope, title: "Health", workspace: records.workspace, now: now, timeZone: .gmt)
        ) {
            BoardFeature()
        } withDependencies: { [now, records] in
            $0.store = .inMemory(records.workspace)
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return store
    }

    private func titles(_ store: TestStoreOf<BoardFeature>) -> [[String]] {
        store.state.columns.map { $0.rows.map(\.title) }
    }

    func testShowsEveryStatusAsAColumn() {
        records.task("Plan long runs", in: health)
        records.task("Buy running shoes", in: health, status: .inProgress)
        records.task("Book physio", in: health, status: .inProgress, idle: 9)
        records.task("Stretch plan", in: health, status: .procrastinated)
        records.task("Gait analysis", in: health, status: .done)
        records.task("Elsewhere", in: records.avenue("HOME", "Home"))

        let store = board(.avenue(health.id))

        XCTAssertEqual(store.state.columns.map(\.id), ["todo", "inProgress", "procrastinated", "done", "wontDo"])
        XCTAssertEqual(
            titles(store),
            [["Plan long runs"], ["Buy running shoes"], ["Book physio", "Stretch plan"], ["Gait analysis"], []])
        let stalled = store.state.columns[2]
        XCTAssertEqual(stalled.rows.compactMap { stalled.notes[$0.id] }, ["idle 9 days", "set by hand"])
    }

    func testShowsOnlyTheProjectsTasks() {
        let marathon = records.project("Half marathon", in: health)
        records.task("Buy running shoes", in: health, project: marathon)
        records.task("Stretch", in: health)

        let store = board(.project(marathon.id))

        XCTAssertEqual(titles(store).flatMap { $0 }, ["Buy running shoes"])
    }

    func testDraggingACardSetsItsStatus() async {
        let shoes = records.task("Buy running shoes", in: health)
        let store = board(.avenue(health.id))

        await store.send(.dropped(shoes.id, on: .procrastinated))
        await store.receive(\.list.saved)

        XCTAssertEqual(titles(store)[2], ["Buy running shoes"])
        let saved = try? await store.dependencies.store.load()
        XCTAssertEqual(saved?.tasks[shoes.id]?.status, .procrastinated)
    }

    func testDroppingOnItsOwnColumnChangesNothing() async {
        let shoes = records.task("Buy running shoes", in: health, status: .inProgress, idle: 9)
        let store = board(.avenue(health.id))

        await store.send(.dropped(shoes.id, on: .procrastinated))

        let saved = try? await store.dependencies.store.load()
        XCTAssertEqual(saved?.tasks[shoes.id]?.status, .inProgress)
    }

    func testDraggingAParentToDoneAsksAboutItsSubtasks() async {
        let shoes = records.task("Buy running shoes", in: health)
        records.task("Measure feet", in: health, parent: shoes)
        let store = board(.avenue(health.id))

        await store.send(.dropped(shoes.id, on: .done))
        await store.receive(\.list.saved)
        XCTAssertNotNil(store.state.list.prompt)

        let saved = try? await store.dependencies.store.load()
        XCTAssertEqual(saved?.tasks[shoes.id]?.status, .todo)
    }

    func testDroppingOntoACardInManualOrderTakesItsPlace() async {
        let plan = records.task("Plan long runs", in: health)
        records.task("Book physio", in: health)
        let shoes = records.task("Buy running shoes", in: health)
        let store = board(.avenue(health.id))
        await store.send(.list(.sortPicked(.manual)))

        await store.send(.droppedCard(shoes.id, onto: plan.id))
        await store.receive(\.list.saved)

        XCTAssertEqual(titles(store)[0], ["Buy running shoes", "Plan long runs", "Book physio"])
        let saved = try? await store.dependencies.store.load()
        let order = saved?.tasks.values.sorted { $0.sortOrder < $1.sortOrder }.map(\.title)
        XCTAssertEqual(order, ["Buy running shoes", "Plan long runs", "Book physio"])
    }

    func testDroppingOntoACardBelowMovesItDown() async {
        let plan = records.task("Plan long runs", in: health)
        records.task("Book physio", in: health)
        let shoes = records.task("Buy running shoes", in: health)
        let store = board(.avenue(health.id))
        await store.send(.list(.sortPicked(.manual)))

        await store.send(.droppedCard(plan.id, onto: shoes.id))
        await store.receive(\.list.saved)

        XCTAssertEqual(titles(store)[0], ["Book physio", "Buy running shoes", "Plan long runs"])
    }

    func testDroppingOntoACardInAnotherColumnSetsItsStatus() async {
        let shoes = records.task("Buy running shoes", in: health)
        let physio = records.task("Book physio", in: health, status: .inProgress)
        let store = board(.avenue(health.id))

        await store.send(.droppedCard(shoes.id, onto: physio.id))
        await store.receive(\.list.saved)

        let saved = try? await store.dependencies.store.load()
        XCTAssertEqual(saved?.tasks[shoes.id]?.status, .inProgress)
    }

    func testDroppingOntoACardInItsColumnOutOfManualOrderChangesNothing() async {
        let plan = records.task("Plan long runs", in: health, priority: .high)
        let shoes = records.task("Buy running shoes", in: health)
        let store = board(.avenue(health.id))
        await store.send(.list(.sortPicked(.smart)))

        await store.send(.droppedCard(shoes.id, onto: plan.id))

        XCTAssertEqual(titles(store)[0], ["Plan long runs", "Buy running shoes"])
    }
}
