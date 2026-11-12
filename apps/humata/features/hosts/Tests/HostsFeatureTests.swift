import ComposableArchitecture
import HumataDaemonClient
import HumataHosts
import XCTest

@MainActor
final class HostsFeatureTests: XCTestCase {
    func testAddingAHostFromTheSshConfigThenInstallingMakesItReachable() async throws {
        let daemon = InMemoryDaemon(sshHosts: ["build-box", "gpu"])
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.addTapped)
        await store.send(\.destination.add.task)
        await store.receive(\.destination.add.sshHostsLoaded)
        XCTAssertEqual(store.state.destination?.add?.sshHosts, ["build-box", "gpu"])
        await store.send(\.destination.add.sshHostPicked, "gpu")
        XCTAssertEqual(store.state.destination?.add?.name, "gpu")
        await store.send(\.destination.add.addTapped)
        await store.receive(\.destination.add.delegate.added)
        XCTAssertNil(store.state.destination)
        XCTAssertEqual(store.state.selection, "gpu")
        await store.receive(\.checked)
        guard case .known(.unreachable) = store.state.health["gpu"] else {
            return XCTFail("a host without Humata's program answered: \(String(describing: store.state.health["gpu"]))")
        }

        await store.send(.installTapped("gpu"))
        XCTAssertEqual(store.state.health["gpu"], .installing)
        await store.receive(\.checked)
        guard case .known(.reachable(let status)) = store.state.health["gpu"] else {
            return XCTFail("installed, gpu still isn't reachable")
        }
        XCTAssertFalse(status.githubToken)
    }

    func testTheListChecksEveryHost() async throws {
        let daemon = InMemoryDaemon()
        for name in ["a", "b"] {
            _ = try await daemon.client.addHost(name, "me@\(name)")
        }
        _ = try await daemon.client.installHost("b")
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.selection, "a")
        await store.receive(\.checked)
        await store.receive(\.checked)
        guard case .known(.unreachable) = store.state.health["a"], case .known(.reachable) = store.state.health["b"]
        else {
            return XCTFail("\(store.state.health)")
        }
    }

    func testVariablesAreSentButTheirValuesNeverShown() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        _ = try await daemon.client.updateHost("far", HostUpdate(setEnv: ["OLD": "1", "KEEP": "2"]))
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        let editor = try XCTUnwrap(store.state.editor)
        XCTAssertEqual(editor.variables.map(\.name), ["KEEP", "OLD"])
        XCTAssertEqual(editor.variables.map(\.value), ["", ""])
        XCTAssertFalse(editor.canSave)

        let old = try XCTUnwrap(editor.variables.first { $0.name == "OLD" })
        await store.send(\.editor.removeVariableTapped, old.id)
        await store.send(\.editor.addVariableTapped)
        let added = try XCTUnwrap(store.state.editor?.variables.last)
        var token = added
        token.name = "GH_TOKEN"
        token.value = "secret"
        var variables = store.state.editor?.variables ?? []
        variables[id: token.id] = token
        await store.send(\.editor.binding.variables, variables)
        await store.send(\.editor.binding.setupScript, "apt-get install -y ripgrep\n")
        await store.send(\.editor.saveTapped)
        await store.receive(\.editor.delegate.saved)
        XCTAssertEqual(daemon.env(of: "far"), ["KEEP": "2", "GH_TOKEN": "secret"])
        XCTAssertEqual(store.state.editor?.variables.map(\.value), ["", ""])
        XCTAssertEqual(store.state.hosts[id: "far"]?.setupScript, "apt-get install -y ripgrep\n")
        await store.receive(\.checked)
        guard case .known(.reachable(let status)) = store.state.health["far"] else {
            return XCTFail("far isn't reachable after saving")
        }
        XCTAssertTrue(status.githubToken)
        XCTAssertEqual(status.setup?.succeeded, true)
    }

    func testThisMacsVariablesAreEditedLikeAHostsWithNoSetupScript() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.receive(\.coordinatorLoaded)
        // With no remote hosts, This Mac is what there is to edit.
        XCTAssertEqual(store.state.selected?.isCoordinator, true)

        await store.send(\.editor.addVariableTapped)
        var variables = store.state.editor?.variables ?? []
        let row = try XCTUnwrap(variables.last)
        variables[id: row.id]?.name = "GH_TOKEN"
        variables[id: row.id]?.value = "secret"
        await store.send(\.editor.binding.variables, variables)
        await store.send(\.editor.saveTapped)
        await store.receive(\.editor.delegate.saved)
        XCTAssertEqual(daemon.env(of: ""), ["GH_TOKEN": "secret"])
        XCTAssertEqual(store.state.coordinator.envNames, ["GH_TOKEN"])
        XCTAssertEqual(store.state.editor?.variables.map(\.value), [""])
        XCTAssertTrue(store.state.hosts.isEmpty)
    }

    func testAVariableNeedsAName() {
        var state = HostEditorFeature.State(host: RemoteHost(name: "far", sshDestination: "me@far"))
        state.variables = [HostEditorFeature.Variable(id: 0, name: "1BAD", value: "x")]
        XCTAssertFalse(state.canSave)
        state.variables = [HostEditorFeature.Variable(id: 0, name: "GOOD_1", value: "x")]
        XCTAssertTrue(state.canSave)
        state.variables = [HostEditorFeature.Variable(id: 0, name: "EMPTY")]
        XCTAssertFalse(state.canSave)
    }

    func testRemovingAHostAsksFirst() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.removeTapped("far"))
        let hosts = try await daemon.client.hosts()
        XCTAssertEqual(hosts.map(\.name), ["far"])
        await store.send(.destination(.presented(.alert(.confirmRemove("far")))))
        await store.receive(\.removed)
        XCTAssertEqual(store.state.hosts, [])
        // With the last remote host gone, This Mac is what's left selected.
        XCTAssertEqual(store.state.selected?.isCoordinator, true)
        let left = try await daemon.client.hosts()
        XCTAssertEqual(left, [])
    }

    func testASessionThatCouldntBeOpenedIsListedAndRemovedAfterAsking() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        let damaged = SkippedSession(
            backend: .claude, id: "5e1f", folder: "/home/me/.humata/claude/sessions/5e1f",
            reason: "the session record is damaged")
        daemon.addSkipped(damaged, on: "far")
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.skippedLoaded)
        XCTAssertEqual(store.state.skipped.elements, [damaged])

        // This Mac skipped none.
        await store.send(.selected(store.state.coordinator.id))
        XCTAssertEqual(store.state.skipped, [])
        await store.receive(\.skippedLoaded)
        XCTAssertEqual(store.state.skipped, [])

        await store.send(.selected("far"))
        await store.receive(\.skippedLoaded)
        await store.send(.removeSkippedTapped(damaged.id))
        let stillThere = try await daemon.client.skipped("far")
        XCTAssertEqual(stillThere, [damaged])
        await store.send(.destination(.presented(.alert(.confirmRemoveSkipped(damaged.id)))))
        await store.receive(\.skippedRemoved)
        XCTAssertEqual(store.state.skipped, [])
        let left = try await daemon.client.skipped("far")
        XCTAssertEqual(left, [])
    }

    func testSessionsAHostLostAreRestoredFromTheirCopies() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        let first = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", host: "far"))
        let second = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", host: "far"))
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.recoveriesLoaded)
        XCTAssertEqual(Set(store.state.recoveries.ids), [first.id, second.id])
        XCTAssertTrue(store.state.recoveries.allSatisfy(\.onHost))

        daemon.lose(host: "far")
        await store.send(.selected("far"))
        await store.receive(\.recoveriesLoaded)
        XCTAssertTrue(store.state.recoveries.allSatisfy { !$0.onHost })

        await store.send(.restoreTapped(first.id))
        XCTAssertEqual(store.state.restoring, [first.id])
        await store.receive(\.restored)
        await store.receive(\.recoveriesLoaded)
        XCTAssertEqual(store.state.restoring, [])
        XCTAssertEqual(store.state.recoveries[id: first.id]?.onHost, true)
        XCTAssertEqual(store.state.recoveries[id: second.id]?.onHost, false)

        await store.send(.restoreAllTapped)
        await store.receive(\.restored)
        await store.receive(\.recoveriesLoaded)
        XCTAssertTrue(store.state.recoveries.allSatisfy(\.onHost))
        let sessions = try await daemon.client.sessions(nil)
        XCTAssertEqual(sessions.filter { $0.host == "far" }.count, 2)
    }

    func testASessionMovedToThisMacLeavesItsHostsCopies() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", host: "far"))
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.recoveriesLoaded)
        await store.send(.moveTapped(session.id, to: nil))
        await store.receive(\.restored)
        await store.receive(\.recoveriesLoaded)
        XCTAssertEqual(store.state.recoveries, [])
        let sessions = try await daemon.client.sessions(nil)
        XCTAssertNil(sessions.first { $0.id == session.id }?.host)
    }

    func testARestoreOntoAnUnreachableHostSaysWhy() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", host: "far"))
        daemon.lose(host: "far")
        daemon.setOffline("far", true)
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.recoveriesLoaded)
        await store.send(.restoreTapped(session.id))
        await store.receive(\.restoreFailed)
        XCTAssertNotNil(store.state.destination?.alert)
        XCTAssertEqual(store.state.restoring, [])
        XCTAssertEqual(store.state.recoveries[id: session.id]?.onHost, false)
    }

    func testARefusedHostSaysWhy() async {
        let store = TestStore(initialState: AddHostFeature.State()) {
            AddHostFeature()
        } withDependencies: {
            $0.daemon = InMemoryDaemon().client
        }
        store.exhaustivity = .off

        await store.send(\.binding.name, "no spaces")
        await store.send(\.binding.destination, "me@far")
        await store.send(.addTapped)
        await store.receive(\.failed)
        XCTAssertNotNil(store.state.alert)
        XCTAssertFalse(store.state.adding)
    }
}
