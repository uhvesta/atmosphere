import ComposableArchitecture
import Foundation
import LifrAvenue
import LifrCore
import LifrStoreClient
import XCTest

@MainActor
final class AvenueFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")
    private lazy var home = records.avenue("HOME", "Home")

    private func dependencies(_ values: inout DependencyValues) {
        values.store = .inMemory(records.workspace)
        values.date.now = now
        values.timeZone = .gmt
        values.uuid = .incrementing
        values.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
    }

    private func avenues() async -> TestStoreOf<AvenuesFeature> {
        let store = TestStore(initialState: AvenuesFeature.State()) {
            AvenuesFeature()
        } withDependencies: {
            self.dependencies(&$0)
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)
        return store
    }

    private func avenue(_ scope: AvenueScope) -> TestStoreOf<AvenueFeature> {
        let store = TestStore(
            initialState: AvenueFeature.State(scope: scope, workspace: records.workspace, now: now, timeZone: .gmt)
        ) {
            AvenueFeature()
        } withDependencies: {
            self.dependencies(&$0)
        }
        store.exhaustivity = .off
        return store
    }

    func testListsAvenuesWithOpenCountsAndActiveProjects() async {
        let marathon = records.project("Half marathon", in: health)
        records.task("Buy running shoes", in: health, project: marathon)
        records.task("Stretch", in: health)
        records.task("Done already", in: health, status: .done)
        records.task("Call the plumber", in: home)

        let store = await avenues()

        XCTAssertEqual(store.state.avenues.map(\.name), ["Health", "Home"])
        XCTAssertEqual(store.state.avenues.map(\.openCount), [2, 1])
        XCTAssertEqual(store.state.avenues[0].projects.map(\.name), ["Half marathon"])
        XCTAssertEqual(store.state.avenues[0].projects.map(\.openCount), [1])
    }

    func testMovingAnAvenueSwapsItWithItsNeighbour() async {
        _ = (health, home)
        let work = records.avenue("WORK", "Work")
        let store = await avenues()

        await store.send(.moveAvenue(health.id, up: false))
        await store.receive(\.moved.success)
        XCTAssertEqual(store.state.avenues.map(\.name), ["Home", "Health", "Work"])

        await store.send(.moveAvenue(work.id, up: true))
        await store.receive(\.moved.success)
        XCTAssertEqual(store.state.avenues.map(\.name), ["Home", "Work", "Health"])
    }

    func testMovingTheFirstAvenueUpChangesNothing() async {
        _ = (health, home)
        let store = await avenues()

        await store.send(.moveAvenue(health.id, up: true))
        XCTAssertEqual(store.state.avenues.map(\.name), ["Health", "Home"])
    }

    func testMovingAProjectUpPutsItFirstInItsAvenue() async {
        records.project("Half marathon", in: health)
        let stretch = records.project("Stretching", in: health)
        let store = await avenues()

        await store.send(.moveProject(stretch.id, up: true))
        await store.receive(\.moved.success)
        XCTAssertEqual(store.state.avenues[0].projects.map(\.name), ["Stretching", "Half marathon"])
    }

    func testGroupsAnAvenueByStatusThenByProject() async {
        let marathon = records.project("Half marathon", in: health)
        records.task("Buy running shoes", in: health, project: marathon)
        records.task("Stretch", in: health, status: .inProgress)
        records.task("Done already", in: health, status: .done)
        let store = avenue(.avenue(health.id))

        XCTAssertEqual(store.state.list.sections.map(\.title), ["In Progress", "To Do", "Done"])

        await store.send(.groupingPicked(.project))

        XCTAssertEqual(store.state.list.sections.map(\.title), ["Half marathon", "No Project"])
        XCTAssertEqual(store.state.list.sections.map { $0.rows.map(\.title) }, [["Buy running shoes"], ["Stretch"]])
    }

    func testAProjectShowsOnlyItsTasks() async {
        let marathon = records.project("Half marathon", in: health)
        records.task("Buy running shoes", in: health, project: marathon)
        records.task("Stretch", in: health)
        let store = avenue(.project(marathon.id))

        XCTAssertEqual(store.state.title, "Half marathon")
        XCTAssertFalse(store.state.canGroupByProject)
        XCTAssertEqual(store.state.list.rows.map(\.title), ["Buy running shoes"])
    }

    func testAddsAProjectToTheAvenue() async {
        _ = health
        let store = avenue(.avenue(health.id))

        await store.send(.newProjectTapped)
        await store.send(.binding(.set(\.newProjectName, "  Half marathon ")))
        await store.send(.newProjectSubmitted)
        await store.receive(\.list.delegate.saved)

        XCTAssertNil(store.state.newProjectName)
        XCTAssertEqual(store.state.list.workspace?.projects.values.map(\.name), ["Half marathon"])
    }

    func testAddsAnAvenueAndShowsIt() async {
        _ = health
        let store = await avenues()

        await store.send(.newAvenueTapped)
        await store.send(.newAvenue(.presented(.binding(.set(\.key, "home")))))
        await store.send(.newAvenue(.presented(.binding(.set(\.name, "Home")))))
        await store.send(.newAvenue(.presented(.addTapped)))
        await store.receive(\.delegate.saved)
        await store.receive(\.newAvenue.dismiss)

        XCTAssertEqual(store.state.avenues.map(\.key), ["HEALTH", "HOME"])
        XCTAssertNil(store.state.newAvenue)
    }

    func testRefusesATakenKeyAndSaysWhy() async {
        _ = health
        let store = await avenues()

        await store.send(.newAvenueTapped)
        await store.send(.newAvenue(.presented(.binding(.set(\.key, "health")))))
        await store.send(.newAvenue(.presented(.addTapped)))
        await store.receive(\.newAvenue.added.failure)

        XCTAssertNotNil(store.state.newAvenue?.error)
        XCTAssertEqual(store.state.avenues.count, 1)
    }

    func testOpensAnAvenueAndKeepsItCurrent() async {
        let stretch = records.task("Stretch", in: health)
        let store = await avenues()

        await store.send(.opened(.avenue(health.id)))
        await store.send(.avenue(.presented(.list(.rowAction(stretch.id, .status(.done))))))
        await store.receive(\.avenue.list.delegate.saved)
        await store.receive(\.avenue.delegate.saved)

        XCTAssertEqual(store.state.avenues.map(\.openCount), [0])
        XCTAssertEqual(store.state.avenue?.list.sections.map(\.title), ["Done"])
    }

    func testCancellingANewAvenueAddsNothing() async {
        _ = health
        let store = await avenues()

        await store.send(.newAvenueTapped)
        await store.send(.newAvenue(.presented(.binding(.set(\.key, "home")))))
        await store.send(.newAvenue(.presented(.cancelTapped)))
        await store.receive(\.newAvenue.dismiss)

        XCTAssertNil(store.state.newAvenue)
        XCTAssertEqual(store.state.avenues.map(\.key), ["HEALTH"])
    }

    func testCancellingAnEditKeepsTheAvenueAsItWas() async throws {
        let store = avenue(.avenue(health.id))

        await store.send(.editAvenueTapped)
        XCTAssertEqual(store.state.editAvenue?.name, "Health")
        await store.send(.editAvenue(.presented(.binding(.set(\.name, "Wellness")))))
        await store.send(.editAvenue(.presented(.cancelTapped)))
        await store.receive(\.editAvenue.dismiss)

        XCTAssertNil(store.state.editAvenue)
        let workspace = try await store.dependencies.store.load()
        XCTAssertEqual(workspace.avenues[health.id]?.name, "Health")
    }
}
