import ComposableArchitecture
import HumataDaemonClient
import HumataHosts
import XCTest

@MainActor
final class EnvironmentsFeatureTests: XCTestCase {
    func testTheSshConfigsHostsListBesideTypedOnes() async throws {
        let daemon = InMemoryDaemon(sshHosts: ["build-box", "gpu"])
        _ = try await daemon.client.addHost("lab", "me@lab.example")
        let store = TestStore(initialState: EnvironmentsFeature.State()) {
            EnvironmentsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.environments.map(\.destination), ["build-box", "gpu", "me@lab.example"])
        XCTAssertEqual(store.state.environments.map { $0.host?.name }, [nil, nil, "lab"])
    }

    func testAddingAConfigHostInstallsHumataThere() async throws {
        let daemon = InMemoryDaemon(sshHosts: ["build-box", "gpu"])
        let store = TestStore(initialState: EnvironmentsFeature.State()) {
            EnvironmentsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.addTapped(destination: "gpu"))
        XCTAssertEqual(store.state.adding, ["gpu"])
        await store.receive(\.added)
        XCTAssertEqual(store.state.selection, "gpu")
        XCTAssertNotNil(store.state.editor)
        await store.receive(\.checked)
        guard case .known(.reachable) = store.state.health["gpu"] else {
            return XCTFail("added, gpu isn't reachable: \(String(describing: store.state.health["gpu"]))")
        }
        XCTAssertEqual(store.state.environments.first { $0.destination == "gpu" }?.host?.name, "gpu")
        XCTAssertTrue(store.state.adding.isEmpty)

        // Adding it again does nothing.
        await store.send(.addTapped(destination: "gpu"))
        XCTAssertTrue(store.state.adding.isEmpty)
    }

    func testAHostOnAnotherBuildIsOfferedUpdate() async throws {
        let daemon = InMemoryDaemon(sshHosts: ["gpu"])
        _ = try await daemon.client.addHost("gpu", "gpu")
        _ = try await daemon.client.installHost("gpu")
        var state = EnvironmentsFeature.State()
        state.bundledVersion = "9.9.9"
        let store = TestStore(initialState: state) {
            EnvironmentsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.receive(\.checked)
        XCTAssertTrue(store.state.needsUpdate("gpu"))

        guard case .known(.reachable(let status))? = store.state.health["gpu"] else {
            return XCTFail("gpu isn't reachable")
        }
        var current = store.state
        current.bundledVersion = status.programVersion
        XCTAssertFalse(current.needsUpdate("gpu"))
    }

    func testAHostTypedInIsAddedAndInstalled() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: EnvironmentsFeature.State()) {
            EnvironmentsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.addOtherTapped)
        await store.send(\.destination.addOther.binding, .set(\.destination, "me@lab.example"))
        await store.send(\.destination.addOther.binding, .set(\.name, "lab"))
        await store.send(\.destination.addOther.addTapped)
        await store.receive(\.destination.addOther.delegate.added)
        XCTAssertNil(store.state.destination)
        await store.receive(\.added)
        await store.receive(\.checked)
        guard case .known(.reachable) = store.state.health["lab"] else {
            return XCTFail("lab isn't reachable after adding it")
        }
        XCTAssertEqual(store.state.environments.map(\.destination), ["me@lab.example"])
    }
}
