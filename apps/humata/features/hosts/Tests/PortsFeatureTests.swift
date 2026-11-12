import ComposableArchitecture
import HumataDaemonClient
import HumataHosts
import XCTest

@MainActor
final class PortsFeatureTests: XCTestCase {
    private func store(_ daemon: InMemoryDaemon, host: String?) -> TestStoreOf<PortsFeature> {
        let store = TestStore(initialState: PortsFeature.State(host: host)) {
            PortsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        return store
    }

    func testAPortThatAnswersIsForwardedListedAndStopped() async {
        let daemon = InMemoryDaemon()
        daemon.listen(on: 5173, at: "build")
        let store = store(daemon, host: "build")

        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.forwards, [])

        await store.send(\.binding.port, "5173")
        await store.send(.forwardTapped)
        await store.receive(\.forwarded)
        let forward = store.state.forwards.first
        XCTAssertEqual(forward?.port, 5173)
        XCTAssertEqual(forward?.localPort, 5173)
        XCTAssertEqual(forward?.url?.absoluteString, "http://127.0.0.1:5173")
        XCTAssertEqual(store.state.port, "")

        // Another host's pane doesn't list it.
        let other = self.store(daemon, host: nil)
        await other.send(.task)
        await other.receive(\.loaded)
        XCTAssertEqual(other.state.forwards, [])

        guard let id = forward?.id else { return XCTFail("nothing forwarded") }
        await store.send(.stopTapped(id))
        await store.receive(\.stopped)
        XCTAssertEqual(store.state.forwards, [])
        let left = try? await daemon.client.forwards()
        XCTAssertEqual(left, [])
    }

    func testAPortNothingAnswersOnSaysSoAndIsntForwarded() async {
        let daemon = InMemoryDaemon()
        let store = store(daemon, host: "build")

        await store.send(\.binding.port, "8080")
        await store.send(.forwardTapped)
        await store.receive(\.failed)
        XCTAssertNotNil(store.state.failure)
        XCTAssertFalse(store.state.forwarding)
        XCTAssertEqual(store.state.forwards, [])
    }

    func testOnlyARealPortCanBeForwarded() async {
        let store = store(InMemoryDaemon(), host: nil)
        await store.send(\.binding.port, "99999")
        XCTAssertNil(store.state.portNumber)
        // Nothing is asked of the daemon.
        await store.send(.forwardTapped)
        XCTAssertFalse(store.state.forwarding)
    }
}
