import ComposableArchitecture
import Foundation
import HumataDaemonClient
import HumataTerminal
import XCTest

@MainActor
final class TerminalsFeatureTests: XCTestCase {
    private func store(_ daemon: InMemoryDaemon, _ state: TerminalsFeature.State) -> TestStoreOf<TerminalsFeature> {
        let store = TestStore(initialState: state) {
            TerminalsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.continuousClock = ImmediateClock()
        }
        store.exhaustivity = .off
        return store
    }

    private func shown(_ store: TestStoreOf<TerminalsFeature>) -> String {
        String(bytes: store.state.current?.screen.data ?? Data(), encoding: .utf8) ?? ""
    }

    func testASessionsTabOpensAShellInItsFolderAndRunsWhatsTyped() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, approvalMode: .allowEverything))
        let store = store(daemon, TerminalsFeature.State(place: HostPlace(sessionID: session.id), opensFirst: true))

        await store.send(.task)
        await store.receive(\.listed)
        await store.receive(\.newTapped)
        await store.receive(\.opened)
        let id = try XCTUnwrap(store.state.selected)
        XCTAssertEqual(store.state.current?.terminal.folder, session.workingDirectory)

        let attach = await store.send(.terminals(.element(id: id, action: .attach)))
        await store.receive(\.terminals)
        await store.send(.terminals(.element(id: id, action: .typed(Data("pwd\r".utf8)))))
        await store.receive(\.terminals)
        XCTAssertTrue(shown(store).contains(session.workingDirectory + "\r\n$ "), shown(store))
        await attach.cancel()
    }

    func testTerminalsStayOpenBetweenViewsAndReplayWhatTheyPrinted() async throws {
        let daemon = InMemoryDaemon()
        let terminal = try await daemon.client.openTerminal(HostPlace(host: "build"), .standard)
        try await daemon.client.typeInTerminal(terminal, Data("echo hi\r".utf8))

        // A host's list doesn't open one by itself, and shows what's running.
        let store = store(daemon, TerminalsFeature.State(place: HostPlace(host: "build"), opensFirst: false))
        await store.send(.task)
        await store.receive(\.listed)
        XCTAssertEqual(store.state.tabs.ids, [terminal.id])

        let attach = await store.send(.terminals(.element(id: terminal.id, action: .attach)))
        await store.receive(\.terminals)
        XCTAssertTrue(shown(store).contains("hi\r\n$ "), shown(store))
        XCTAssertEqual(store.state.current?.screen.generation, 1)
        await attach.cancel()
    }

    func testExitingEndsTheShellAndClosingForgetsIt() async throws {
        let daemon = InMemoryDaemon()
        let store = store(daemon, TerminalsFeature.State(place: HostPlace(), opensFirst: false))
        await store.send(.task)
        await store.receive(\.listed)
        XCTAssertTrue(store.state.tabs.isEmpty)

        await store.send(.newTapped)
        await store.receive(\.opened)
        let id = try XCTUnwrap(store.state.selected)
        let attach = await store.send(.terminals(.element(id: id, action: .attach)))
        await store.receive(\.terminals)
        await store.send(.terminals(.element(id: id, action: .typed(Data("exit 3\r".utf8)))))
        await store.receive(\.terminals)
        await store.receive(\.terminals)
        XCTAssertEqual(store.state.current?.terminal.exitCode, 3)
        await attach.finish()

        await store.send(.closeTapped(id))
        await store.receive(\.closed)
        XCTAssertNil(store.state.selected)
        let left = try await daemon.client.terminals(nil)
        XCTAssertTrue(left.isEmpty)
    }

    func testATerminalClosedElsewhereStopsReconnecting() async throws {
        let daemon = InMemoryDaemon()
        let terminal = try await daemon.client.openTerminal(HostPlace(), .standard)
        let store = store(daemon, TerminalsFeature.State(place: HostPlace(), opensFirst: false))
        await store.send(.task)
        await store.receive(\.listed)
        try await daemon.client.closeTerminal(terminal)

        await store.send(.terminals(.element(id: terminal.id, action: .attach)))
        await store.receive(\.terminals)
        await store.receive(\.terminals)
        XCTAssertTrue(store.state.current?.exited ?? false)
        XCTAssertFalse(store.state.current?.detached ?? true)
    }
}
