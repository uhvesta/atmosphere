import ComposableArchitecture
import HumataDaemonClient
import HumataHosts
import XCTest

@MainActor
final class AgentAccessFeatureTests: XCTestCase {
    private func store(_ daemon: InMemoryDaemon) -> TestStoreOf<AgentAccessFeature> {
        let store = TestStore(initialState: AgentAccessFeature.State()) {
            AgentAccessFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        return store
    }

    func testReadsAndChangesAreOnAndDestroysOffUntilSwitched() async throws {
        let daemon = InMemoryDaemon()
        let store = store(daemon)

        await store.send(.task)
        await store.receive(\.loaded)
        for client in AccessClient.allCases {
            let access = try XCTUnwrap(store.state.access)[client]
            XCTAssertTrue(access.enabled)
            XCTAssertTrue(access[.read])
            XCTAssertTrue(access[.change])
            XCTAssertFalse(access[.destroy])
        }
    }

    func testTheMainSwitchIsSavedForItsClientAlone() async throws {
        let daemon = InMemoryDaemon()
        let store = store(daemon)
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.mainSwitchToggled(.agents, false))
        XCTAssertTrue(store.state.saving)
        await store.receive(\.saved)
        XCTAssertFalse(store.state.saving)
        let saved = try await daemon.client.agentAccess()
        XCTAssertFalse(saved.agents.enabled)
        XCTAssertTrue(saved.terminal.enabled)
    }

    func testAGroupOffTurnsItsActionsOffAndKeepsTheirOwnSwitches() async throws {
        let daemon = InMemoryDaemon()
        let store = store(daemon)
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.groupToggled(.terminal, .change, false))
        await store.receive(\.saved)
        let saved = try await daemon.client.agentAccess()
        let changes = saved.terminal.actions.filter { $0.group == .change }
        XCTAssertFalse(changes.isEmpty)
        for action in changes {
            XCTAssertTrue(action.enabled)
            XCTAssertFalse(saved.terminal.allows(action))
        }
        // Another group's actions, and the agents', still run.
        let reads = saved.terminal.actions.filter { $0.group == .read }
        XCTAssertTrue(reads.allSatisfy { saved.terminal.allows($0) })
        XCTAssertTrue(saved.agents[.change])
    }

    func testAnActionsOwnSwitchIsSaved() async throws {
        let daemon = InMemoryDaemon()
        let store = store(daemon)
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.groupToggled(.agents, .destroy, true))
        await store.receive(\.saved)
        await store.send(.actionToggled(.agents, id: "delete_project", false))
        await store.receive(\.saved)

        let agents = try await daemon.client.agentAccess().agents
        let delete = try XCTUnwrap(agents.actions.first { $0.id == "delete_project" })
        let archive = try XCTUnwrap(agents.actions.first { $0.id == "archive_session" })
        XCTAssertFalse(agents.allows(delete))
        XCTAssertTrue(agents.allows(archive))
    }

    func testAChangeThatWasntSavedGoesBackAndSaysWhy() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: AgentAccessFeature.State()) {
            AgentAccessFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.setAgentAccess = { _ in throw DaemonError.disconnected }
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.groupToggled(.agents, .destroy, true))
        XCTAssertEqual(store.state.access?.agents.destroy, true)
        await store.receive(\.saveFailed)
        XCTAssertEqual(store.state.access?.agents.destroy, false)
        XCTAssertFalse(store.state.saving)
        XCTAssertNotNil(store.state.failure)
    }

    func testSettingsThatCantBeReadSayWhyAndCanBeTriedAgain() async throws {
        let daemon = InMemoryDaemon()
        let failing = LockIsolated(true)
        let store = TestStore(initialState: AgentAccessFeature.State()) {
            AgentAccessFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.agentAccess = {
                if failing.value { throw DaemonError.disconnected }
                return AgentAccess.standard
            }
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loadFailed)
        XCTAssertNil(store.state.access)
        XCTAssertNotNil(store.state.failure)

        failing.setValue(false)
        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertNil(store.state.failure)
        XCTAssertEqual(store.state.access, AgentAccess.standard)
    }
}
