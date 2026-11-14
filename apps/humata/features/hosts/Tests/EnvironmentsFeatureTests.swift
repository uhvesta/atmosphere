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
        await store.receive(\.installed)
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
        await store.receive(\.installed)
        guard case .known(.reachable) = store.state.health["lab"] else {
            return XCTFail("lab isn't reachable after adding it")
        }
        XCTAssertEqual(store.state.environments.map(\.destination), ["me@lab.example"])
    }

    func testACheckThatAnswersDuringAnInstallLeavesItInstalling() async throws {
        let daemon = InMemoryDaemon(sshHosts: ["gpu"])
        _ = try await daemon.client.addHost("gpu", "gpu")
        let (installAsked, asked) = AsyncStream.makeStream(of: Void.self)
        let (installAnswers, answer) = AsyncStream.makeStream(of: Void.self)
        let installs = LockIsolated(0)
        var client = daemon.client
        let install = client.installHost
        client.installHost = { host in
            installs.withValue { $0 += 1 }
            asked.yield()
            for await _ in installAnswers { break }
            return try await install(host)
        }
        let store = TestStore(initialState: EnvironmentsFeature.State()) {
            EnvironmentsFeature()
        } withDependencies: {
            $0.daemon = client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.receive(\.checked)
        await store.send(.updateTapped("gpu"))
        for await _ in installAsked { break }

        // The pane shown again asks after every host while the install runs.
        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.health["gpu"], .installing)
        await store.send(.checked("gpu", .unreachable("Humata's program isn't on gpu yet.")))
        XCTAssertEqual(store.state.health["gpu"], .installing)
        await store.send(.updateTapped("gpu"))
        XCTAssertEqual(installs.value, 1)

        answer.yield()
        await store.receive(\.installed)
        guard case .known(.reachable) = store.state.health["gpu"] else {
            return XCTFail("installed, gpu still isn't reachable: \(String(describing: store.state.health["gpu"]))")
        }
    }

    func testWhatWasChangedOnAHostIsStillThereAfterLookingAtAnother() async throws {
        let daemon = InMemoryDaemon(sshHosts: ["gpu"])
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.addHost("near", "me@near")
        let store = TestStore(initialState: EnvironmentsFeature.State()) {
            EnvironmentsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.selected("far"))
        await store.send(\.editor.binding.setupScript, "apt-get install -y ripgrep\n")

        await store.send(.selected("near"))
        XCTAssertEqual(store.state.editor?.host.id, "near")
        XCTAssertNil(store.state.editor?.update)
        await store.send(.selected(nil))
        XCTAssertNil(store.state.editor)

        await store.send(.selected("far"))
        XCTAssertEqual(store.state.editor?.host.id, "far")
        XCTAssertEqual(store.state.editor?.setupScript, "apt-get install -y ripgrep\n")

        // A host added meanwhile doesn't take its place.
        await store.send(.addTapped(destination: "gpu"))
        await store.receive(\.added)
        XCTAssertEqual(store.state.selection, "far")
        XCTAssertEqual(store.state.editor?.setupScript, "apt-get install -y ripgrep\n")

        // Saved, there's nothing left to bring back.
        await store.send(\.editor.saveTapped)
        await store.receive(\.editor.delegate.saved)
        await store.send(.selected("near"))
        await store.send(.selected("far"))
        XCTAssertNil(store.state.editor?.update)
        XCTAssertEqual(store.state.editor?.setupScript, "apt-get install -y ripgrep\n")
        await store.skipReceivedActions(strict: false)
    }
}
