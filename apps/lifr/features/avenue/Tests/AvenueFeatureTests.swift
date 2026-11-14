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

    func testAvenuesThatFailToLoadCanBeLoadedAgain() async {
        struct Unreadable: Error {}
        let failing = LockIsolated(true)
        let stored = StoreClient.inMemory(records.workspace)
        let store = TestStore(initialState: AvenuesFeature.State()) {
            AvenuesFeature()
        } withDependencies: {
            self.dependencies(&$0)
            $0.store.load = {
                if failing.value { throw Unreadable() }
                return try await stored.load()
            }
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertNotNil(store.state.loadError)
        XCTAssertNil(store.state.workspace)

        failing.setValue(false)
        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertNil(store.state.loadError)
        XCTAssertNotNil(store.state.workspace)
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

    func testTheInboxStaysFirstAndNothingTakesItsPlace() async {
        let inbox = records.inbox()
        _ = (health, home)
        let store = await avenues()
        XCTAssertEqual(store.state.inbox?.id, inbox.id)
        XCTAssertEqual(store.state.movable.map(\.name), ["Health", "Home"])

        await store.send(.moveAvenue(health.id, up: true))
        await store.send(.avenueDropped(home.id, onto: inbox.id))
        await store.send(.moveAvenue(inbox.id, up: false))
        await store.send(.avenueDropped(inbox.id, onto: home.id))
        XCTAssertEqual(store.state.avenues.map(\.name), ["Inbox", "Health", "Home"])
    }

    func testMovingAProjectUpPutsItFirstInItsAvenue() async {
        records.project("Half marathon", in: health)
        let stretch = records.project("Stretching", in: health)
        let store = await avenues()

        await store.send(.moveProject(stretch.id, up: true))
        await store.receive(\.moved.success)
        XCTAssertEqual(store.state.avenues[0].projects.map(\.name), ["Stretching", "Half marathon"])
    }

    func testDraggingAnAvenueOntoAnotherTakesItsPlace() async {
        _ = (health, home)
        let work = records.avenue("WORK", "Work")
        let store = await avenues()

        await store.send(.avenueDropped(health.id, onto: work.id))
        await store.receive(\.moved.success)
        XCTAssertEqual(store.state.avenues.map(\.name), ["Home", "Work", "Health"])

        await store.send(.avenueDropped(work.id, onto: home.id))
        await store.receive(\.moved.success)
        XCTAssertEqual(store.state.avenues.map(\.name), ["Work", "Home", "Health"])
    }

    func testDraggingAProjectOntoOneInAnotherAvenueChangesNothing() async {
        let marathon = records.project("Half marathon", in: health)
        let garden = records.project("Garden", in: home)
        let store = await avenues()

        await store.send(.projectDropped(marathon.id, onto: garden.id))
        XCTAssertEqual(store.state.avenues.map { $0.projects.map(\.name) }, [["Half marathon"], ["Garden"]])
    }

    func testDraggingAProjectOntoAnotherInItsAvenueTakesItsPlace() async {
        records.project("Half marathon", in: health)
        let stretch = records.project("Stretching", in: health)
        let yoga = records.project("Yoga", in: health)
        let store = await avenues()

        await store.send(.projectDropped(yoga.id, onto: stretch.id))
        await store.receive(\.moved.success)
        XCTAssertEqual(store.state.avenues[0].projects.map(\.name), ["Half marathon", "Yoga", "Stretching"])
    }

    func testATaskThatStartsLaterIsInItsOwnSectionUntilThatDay() async {
        records.task("Paint the fence", in: home, starts: 2)
        records.task("Sort the garage", in: home)
        let store = avenue(.avenue(home.id))

        let sections = store.state.list.sections.map { ($0.id, $0.rows.map(\.title)) }
        XCTAssertEqual(sections.first { $0.0 == "later" }?.1, ["Paint the fence"])
        XCTAssertEqual(sections.first { $0.0 == "todo" }?.1, ["Sort the garage"])
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

    func testShowCompletedOffHidesClosedTasks() async {
        records.task("Stretch", in: health)
        records.task("Done already", in: health, status: .done)
        records.task("Gave up", in: health, status: .wontDo)
        let store = avenue(.avenue(health.id))

        await store.send(.showsCompletedToggled(false))
        XCTAssertEqual(store.state.list.rows.map(\.title), ["Stretch"])

        await store.send(.showsCompletedToggled(true))
        XCTAssertEqual(store.state.list.sections.map(\.title), ["To Do", "Done", "Won't Do"])
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

    func testARefusedProjectNameIsWhereTheNextNewProjectStarts() async {
        _ = health
        let store = avenue(.avenue(health.id))
        let long = String(repeating: "a", count: 201)
        store.dependencies.store.save = { _ in throw Refused() }

        await store.send(.newProjectTapped)
        await store.send(.binding(.set(\.newProjectName, long)))
        await store.send(.newProjectSubmitted)
        await store.receive(\.newProjectRefused)
        await store.receive(\.list.saved)
        XCTAssertNotNil(store.state.list.alert)

        await store.send(.newProjectTapped)
        XCTAssertEqual(store.state.newProjectName, long)
        XCTAssertEqual(store.state.list.workspace?.projects.isEmpty, true)
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

private struct Refused: Error {}
