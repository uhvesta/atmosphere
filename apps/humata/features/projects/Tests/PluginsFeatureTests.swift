import ComposableArchitecture
import HumataDaemonClient
import HumataProjects
import XCTest

@MainActor
final class PluginsFeatureTests: XCTestCase {
    private let review = Plugin(
        agent: .claude, pluginID: "review@market", description: "Reviews diffs", version: "1.2.0", enabled: true,
        skills: ["review"], mcpServers: ["github"])
    private let lint = Plugin(agent: .copilot, pluginID: "lint", host: "builder", enabled: false, hooks: true)

    private func daemon() -> InMemoryDaemon {
        let daemon = InMemoryDaemon()
        daemon.setPlugins(PluginList(plugins: [review, lint], unreachable: ["laptop"]))
        return daemon
    }

    func testListingShowsEachHostsPluginsAndPicksTheFirstProject() async throws {
        let daemon = daemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let store = TestStore(initialState: PluginsFeature.State()) {
            PluginsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        await store.send(.task)
        XCTAssertTrue(store.state.loading)
        await store.receive(\.loaded)
        XCTAssertFalse(store.state.loading)
        XCTAssertEqual(store.state.list?.plugins.count, 2)
        XCTAssertEqual(store.state.list?.unreachable, ["laptop"])
        XCTAssertEqual(store.state.projectID, project.id)
        XCTAssertEqual(store.state.choice(review), .cli)
    }

    func testAChoiceHoldsAndGoesBackToTheCLIs() async throws {
        let daemon = daemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let store = TestStore(initialState: PluginsFeature.State()) {
            PluginsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.choiceChanged(review, .off))
        await store.receive(\.choiceSaved)
        XCTAssertEqual(store.state.choice(review), .off)
        XCTAssertTrue(store.state.saving.isEmpty)
        let saved = try await daemon.client.projects().first { $0.id == project.id }
        XCTAssertEqual(saved?.plugin(.claude, "review@market"), false)

        await store.send(.choiceChanged(review, .cli))
        await store.receive(\.choiceSaved)
        XCTAssertEqual(store.state.choice(review), .cli)
        let back = try await daemon.client.projects().first { $0.id == project.id }
        XCTAssertNil(back?.plugin(.claude, "review@market"))
    }

    func testAChoiceForAProjectGoneFailsAndListsAgain() async throws {
        let daemon = daemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let store = TestStore(initialState: PluginsFeature.State()) {
            PluginsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)
        try await daemon.client.deleteProject(project.id)

        await store.send(.choiceChanged(review, .on))
        await store.receive(\.choiceFailed)
        XCTAssertTrue(store.state.loading)
        await store.receive(\.loaded)
        XCTAssertFalse(store.state.loading)
        XCTAssertNotNil(store.state.failure)
        XCTAssertNil(store.state.projectID)
    }

    func testAFirstListingThatFailsShowsNoListUntilOneIsRead() async throws {
        let daemon = daemon()
        let client = daemon.client
        let fails = LockIsolated(true)
        let store = TestStore(initialState: PluginsFeature.State()) {
            PluginsFeature()
        } withDependencies: {
            $0.daemon = client
            $0.daemon.plugins = {
                if fails.value { throw DaemonError.disconnected }
                return try await client.plugins()
            }
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.failed)
        XCTAssertNil(store.state.list, "a list that couldn't be read isn't an empty one")
        XCTAssertNotNil(store.state.failure)
        XCTAssertFalse(store.state.loading)

        fails.setValue(false)
        await store.send(.refreshTapped)
        await store.receive(\.loaded)
        XCTAssertNil(store.state.failure)
        XCTAssertEqual(store.state.list?.plugins.count, 2)

        // One that fails after keeps what was listed.
        fails.setValue(true)
        await store.send(.refreshTapped)
        await store.receive(\.failed)
        XCTAssertEqual(store.state.list?.plugins.count, 2)
    }
}
