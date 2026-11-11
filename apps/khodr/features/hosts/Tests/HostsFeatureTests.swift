import ComposableArchitecture
import KhodrDaemonClient
import KhodrHosts
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
            return XCTFail("a host without Khodr's program answered: \(String(describing: store.state.health["gpu"]))")
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
        XCTAssertNil(store.state.editor)
        let left = try await daemon.client.hosts()
        XCTAssertEqual(left, [])
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
