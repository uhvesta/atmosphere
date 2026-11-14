import ComposableArchitecture
import HumataDaemonClient
import HumataSession
import XCTest

@MainActor
final class PlanPanelTests: XCTestCase {
    /// A project with "design", "build" and "ship", ship waiting on build
    /// and build on design.
    private func project() async throws -> (InMemoryDaemon, Project, [Item]) {
        let daemon = InMemoryDaemon(script: { _ in [] })
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let ship = try await daemon.client.createItem(project.id, ItemFields(title: "ship"))
        let build = try await daemon.client.createItem(project.id, ItemFields(title: "build"))
        let design = try await daemon.client.createItem(project.id, ItemFields(title: "design"))
        _ = try await daemon.client.updateItem(ship.id, ItemUpdate(link: [build.id]))
        _ = try await daemon.client.updateItem(build.id, ItemUpdate(link: [design.id]))
        return (daemon, project, [design, build, ship])
    }

    func testThePlanListsTheItemsAfterWhatTheyWaitOnAndIsReadAgainOnlyForSomethingNew() async throws {
        let (daemon, project, items) = try await project()
        let clock = TestClock()
        let store = TestStore(initialState: PlanPanelFeature.State(projectID: project.id, itemID: nil)) {
            PlanPanelFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.continuousClock = clock
        }
        let listed = try await daemon.client.items(project.id).sorted { $0.planOrder < $1.planOrder }
        XCTAssertEqual(listed.map(\.title), ["design", "build", "ship"])
        XCTAssertEqual(listed.map(\.blocked), [false, true, true])

        let task = await store.send(.task)
        await store.receive(\.loaded) {
            $0.loaded = true
            $0.planned = listed
        }

        // Nothing changed, so the next read sends nothing: were it sent,
        // it would be the one received below, with the plan unchanged.
        await clock.advance(by: .seconds(2))

        _ = try await daemon.client.updateItem(items[0].id, ItemUpdate(status: .done))
        let after = try await daemon.client.items(project.id).sorted { $0.planOrder < $1.planOrder }
        XCTAssertEqual(after.map(\.blocked), [false, false, true])
        await clock.advance(by: .seconds(2))
        await store.receive(\.loaded) {
            $0.planned = after
        }
        await task.cancel()
    }

    func testAPlanThatCantBeReadSaysWhy() async throws {
        let (daemon, project, _) = try await project()
        let clock = TestClock()
        let store = TestStore(initialState: PlanPanelFeature.State(projectID: project.id, itemID: nil)) {
            PlanPanelFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.items = { _ in throw DaemonError.disconnected }
            $0.continuousClock = clock
        }
        store.exhaustivity = .off

        let task = await store.send(.task)
        await store.receive(\.failed)
        XCTAssertNotNil(store.state.failure)
        XCTAssertTrue(store.state.planned.isEmpty)
        await task.cancel()
    }

    func testASessionInNoProjectReadsNoPlan() async throws {
        let store = TestStore(initialState: PlanPanelFeature.State(projectID: nil, itemID: nil)) {
            PlanPanelFeature()
        } withDependencies: {
            $0.daemon.items = { _ in
                XCTFail("read a plan for a session in no project")
                return []
            }
        }
        await store.send(.task)
    }

    func testShowingThePlanIsRememberedForTheNextSession() async throws {
        let store = TestStore(initialState: PlanPanelFeature.State(projectID: "p", itemID: nil)) {
            PlanPanelFeature()
        }
        store.exhaustivity = .off
        let shown = store.state.shown

        await store.send(.toggleTapped)
        XCTAssertEqual(store.state.shown, !shown)
        XCTAssertEqual(PlanPanelFeature.State(projectID: "q", itemID: nil).shown, !shown)

        await store.send(.toggleTapped)
        XCTAssertEqual(PlanPanelFeature.State(projectID: "q", itemID: nil).shown, shown)
    }

    func testTappingAnItemOpensItOnTheBoard() async throws {
        let store = TestStore(initialState: PlanPanelFeature.State(projectID: "p", itemID: nil)) {
            PlanPanelFeature()
        }
        store.exhaustivity = .off
        await store.send(.itemTapped("i1"))
        await store.receive(\.delegate.openItem, "i1")
    }
}
