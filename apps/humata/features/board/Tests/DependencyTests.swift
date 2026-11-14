import ComposableArchitecture
import HumataBoard
import HumataDaemonClient
import XCTest

@MainActor
final class DependencyTests: XCTestCase {
    private struct Plan {
        var daemon: InMemoryDaemon
        var project: Project
        var design: Item
        var build: Item
        var ship: Item
    }

    /// A project with the tasks "design", "build" and "ship", added in that
    /// order, none waiting on another.
    private func plan() async throws -> Plan {
        let daemon = InMemoryDaemon(script: { _ in [] })
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let design = try await daemon.client.createItem(project.id, ItemFields(title: "design"))
        let build = try await daemon.client.createItem(project.id, ItemFields(title: "build"))
        let ship = try await daemon.client.createItem(project.id, ItemFields(title: "ship"))
        return Plan(daemon: daemon, project: project, design: design, build: build, ship: ship)
    }

    private func page(_ plan: Plan, of item: Item) async throws -> TestStoreOf<ItemFeature> {
        let board = try await plan.daemon.client.items(plan.project.id)
        let store = TestStore(
            initialState: ItemFeature.State(
                project: plan.project, item: item, parent: nil, subtasks: [], sessions: [], board: board)
        ) {
            ItemFeature()
        } withDependencies: {
            $0.daemon = plan.daemon.client
        }
        store.exhaustivity = .off
        return store
    }

    private func board(_ plan: Plan) -> TestStoreOf<BoardFeature> {
        let store = TestStore(initialState: BoardFeature.State(project: plan.project)) {
            BoardFeature()
        } withDependencies: {
            $0.daemon = plan.daemon.client
        }
        store.exhaustivity = .off
        return store
    }

    private func item(_ plan: Plan, _ id: Item.ID) async throws -> Item? {
        try await plan.daemon.client.items(plan.project.id).first { $0.id == id }
    }

    func testAnItemLinkedToAnotherIsBlockedUntilThatIsDone() async throws {
        let plan = try await plan()
        let page = try await page(plan, of: plan.build)
        XCTAssertTrue(page.state.linkable.contains { $0.id == plan.design.id })

        await page.send(.linkPicked(plan.design.id))
        await page.receive(\.updated)
        XCTAssertEqual(page.state.item.dependsOn, [plan.design.id])
        XCTAssertEqual(page.state.dependencies.map(\.id), [plan.design.id])
        XCTAssertFalse(page.state.linkable.contains { $0.id == plan.design.id })
        let blocked = try await item(plan, plan.build.id)
        XCTAssertEqual(blocked?.blocked, true)
        XCTAssertEqual(blocked?.waitingOn, [plan.design.id])

        _ = try await plan.daemon.client.updateItem(plan.design.id, ItemUpdate(status: .done))
        let ready = try await item(plan, plan.build.id)
        XCTAssertEqual(ready?.blocked, false)
        XCTAssertEqual(ready?.waitingOn, [])
    }

    func testUnlinkingAnItemEndsTheWait() async throws {
        let plan = try await plan()
        _ = try await plan.daemon.client.updateItem(plan.build.id, ItemUpdate(link: [plan.design.id]))
        let linked = try await item(plan, plan.build.id)
        let page = try await page(plan, of: try XCTUnwrap(linked))

        await page.send(.unlinkTapped(plan.design.id))
        await page.receive(\.updated)
        XCTAssertEqual(page.state.item.dependsOn, [])
        let unlinked = try await item(plan, plan.build.id)
        XCTAssertEqual(unlinked?.blocked, false)
    }

    func testALinkThatClosesALoopIsRefusedAndLeavesTheItemAsItWas() async throws {
        let plan = try await plan()
        _ = try await plan.daemon.client.updateItem(plan.build.id, ItemUpdate(link: [plan.design.id]))
        _ = try await plan.daemon.client.updateItem(plan.ship.id, ItemUpdate(link: [plan.build.id]))
        let page = try await page(plan, of: plan.design)

        await page.send(.linkPicked(plan.ship.id))
        await page.receive(\.failed)
        XCTAssertNotNil(page.state.destination?.alert)
        XCTAssertEqual(page.state.item.dependsOn, [])
        let design = try await item(plan, plan.design.id)
        XCTAssertEqual(design?.dependsOn, [])
    }

    func testALinkToItselfOrToAnotherProjectsItemIsRefused() async throws {
        let plan = try await plan()
        let other = try await plan.daemon.client.createProject("other", [Repository(path: "/code/other")], "")
        let elsewhere = try await plan.daemon.client.createItem(other.id, ItemFields(title: "elsewhere"))

        for on in [plan.design.id, elsewhere.id] {
            do {
                _ = try await plan.daemon.client.updateItem(plan.design.id, ItemUpdate(link: [on]))
                XCTFail("linked \(plan.design.id) to \(on)")
            } catch DaemonError.refused(let code, _) {
                XCTAssertEqual(code, .invalidDependency)
            }
        }
        let design = try await item(plan, plan.design.id)
        XCTAssertEqual(design?.dependsOn, [])
    }

    func testThePlanPutsEachItemAfterWhatItWaitsOn() async throws {
        let plan = try await plan()
        // design waits on ship, so ship comes first; build has no links and
        // keeps its place among the rest by when it was added.
        _ = try await plan.daemon.client.updateItem(plan.design.id, ItemUpdate(link: [plan.ship.id]))
        let store = board(plan)
        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.planned.map(\.title), ["build", "ship", "design"])
    }

    func testAMilestonesTasksWaitOnWhatItDependsOnAndDependingOnItWaitsForThem() async throws {
        let plan = try await plan()
        let milestone = try await plan.daemon.client.createItem(
            plan.project.id, ItemFields(title: "launch", kind: .milestone, dependsOn: [plan.design.id]))
        let task = try await plan.daemon.client.createItem(
            plan.project.id, ItemFields(parentID: milestone.id, title: "announce"))
        _ = try await plan.daemon.client.updateItem(plan.ship.id, ItemUpdate(link: [milestone.id]))

        let waiting = try await item(plan, task.id)
        XCTAssertEqual(waiting?.waitingOn, [plan.design.id])
        let shipping = try await item(plan, plan.ship.id)
        XCTAssertEqual(shipping?.waitingOn, [milestone.id])

        _ = try await plan.daemon.client.updateItem(plan.design.id, ItemUpdate(status: .done))
        _ = try await plan.daemon.client.updateItem(task.id, ItemUpdate(status: .done))
        let shipped = try await item(plan, plan.ship.id)
        XCTAssertEqual(shipped?.blocked, false, "a milestone whose tasks are all done is finished")

        // The milestone waits on its task, so the task can't wait on ship.
        do {
            _ = try await plan.daemon.client.updateItem(task.id, ItemUpdate(link: [plan.ship.id]))
            XCTFail("linked a milestone's task to what waits on the milestone")
        } catch DaemonError.refused(let code, _) {
            XCTAssertEqual(code, .dependencyCycle)
        }
    }

    func testRevertingALinkTakesItBack() async throws {
        let plan = try await plan()
        _ = try await plan.daemon.client.updateItem(plan.build.id, ItemUpdate(link: [plan.design.id]))
        let log = try await plan.daemon.client.boardLog(plan.project.id, nil, nil)
        let change = try XCTUnwrap(log.first)
        _ = try await plan.daemon.client.revert(change.id)
        let build = try await item(plan, plan.build.id)
        XCTAssertEqual(build?.dependsOn, [])
    }

    func testAnOutsideAgentsProposedDependencyLinksOnlyOnceApplied() async throws {
        let plan = try await plan()
        plan.daemon.proposeFromOutside(
            Proposal(id: "p1", kind: .dependency, title: "", itemID: plan.ship.id, dependsOn: [plan.build.id]),
            for: plan.project.id)
        let store = board(plan)
        await store.send(.proposals(.refresh))
        await store.receive(\.proposals.listed)
        XCTAssertEqual(store.state.proposals.pending.first?.dependsOnTitles, ["build"])
        let before = try await item(plan, plan.ship.id)
        XCTAssertEqual(before?.dependsOn, [])

        await store.send(.proposals(.applyTapped("p1")))
        await store.receive(\.proposals.decided)
        let after = try await item(plan, plan.ship.id)
        XCTAssertEqual(after?.dependsOn, [plan.build.id])
        await store.finish()
    }

    func testTheBoardShowsAsTheGraphOnceItIsPicked() async throws {
        let plan = try await plan()
        let store = board(plan)
        XCTAssertEqual(store.state.layout, .columns)
        await store.send(.layoutPicked(.graph))
        XCTAssertEqual(store.state.layout, .graph)
    }
}
