import ComposableArchitecture
import Foundation
import HumataDaemonClient
import HumataTerminal
import XCTest

@MainActor
final class TerminalsFeatureTests: XCTestCase {
    private func store(
        _ daemon: InMemoryDaemon, _ state: TerminalsFeature.State,
        dependencies: @escaping (inout DependencyValues) -> Void = { _ in }
    ) -> TestStoreOf<TerminalsFeature> {
        let store = TestStore(initialState: state) {
            TerminalsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            dependencies(&$0)
        }
        store.exhaustivity = .off
        return store
    }

    func testASessionsTabOpensAShellInItsFolderAndDrawsItWithItsCommand() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, approvalMode: .allowEverything))
        let store = store(daemon, TerminalsFeature.State(place: HostPlace(sessionID: session.id), opensFirst: true)) {
            $0.terminalCommand = TerminalCommandClient { TerminalCommand(line: "show \($0.id)") }
        }

        await store.send(.task)
        await store.receive(\.listed)
        await store.receive(\.newTapped)
        await store.receive(\.opened)
        let id = try XCTUnwrap(store.state.selected)
        XCTAssertEqual(store.state.current?.terminal.folder, session.workingDirectory)
        XCTAssertNil(store.state.current?.command)

        await store.send(.terminals(.element(id: id, action: .attach)))
        XCTAssertEqual(store.state.current?.command, TerminalCommand(line: "show \(id)"))
    }

    func testTerminalsStayOpenBetweenViews() async throws {
        let daemon = InMemoryDaemon()
        let terminal = try await daemon.client.openTerminal(HostPlace(host: "build"), .standard)

        // A host's list doesn't open one by itself, and shows what's running.
        let store = store(daemon, TerminalsFeature.State(place: HostPlace(host: "build"), opensFirst: false))
        await store.send(.task)
        await store.receive(\.listed)
        XCTAssertEqual(store.state.tabs.ids, [terminal.id])
        XCTAssertEqual(store.state.selected, terminal.id)
    }

    func testAListThatCouldntBeReadSaysWhyUntilItsReadAgain() async throws {
        let daemon = InMemoryDaemon()
        let terminal = try await daemon.client.openTerminal(HostPlace(host: "build"), .standard)
        let answers = LockIsolated(false)
        let store = store(daemon, TerminalsFeature.State(place: HostPlace(host: "build"), opensFirst: false)) {
            let list = $0.daemon.terminals
            $0.daemon.terminals = { host in
                guard answers.value else { throw DaemonError.unexpected("no answer") }
                return try await list(host)
            }
        }

        await store.send(.task)
        await store.receive(\.failed)
        XCTAssertNotNil(store.state.listFailure)
        XCTAssertFalse(store.state.listed)

        answers.setValue(true)
        await store.send(.task)
        XCTAssertNil(store.state.listFailure)
        await store.receive(\.listed)
        XCTAssertNil(store.state.listFailure)
        XCTAssertEqual(store.state.tabs.ids, [terminal.id])
    }

    func testAShellThatExitedShowsItsCodeAndClosingForgetsIt() async throws {
        let daemon = InMemoryDaemon()
        let store = store(daemon, TerminalsFeature.State(place: HostPlace(), opensFirst: false))
        await store.send(.task)
        await store.receive(\.listed)
        XCTAssertTrue(store.state.tabs.isEmpty)

        await store.send(.newTapped)
        await store.receive(\.opened)
        let id = try XCTUnwrap(store.state.selected)
        let terminal = try XCTUnwrap(store.state.current?.terminal)
        try await daemon.client.typeInTerminal(terminal, Data("exit 3\r".utf8))

        await store.send(.terminals(.element(id: id, action: .surfaceExited)))
        await store.receive(\.terminals[id: id].checked)
        XCTAssertEqual(store.state.current?.terminal.exitCode, 3)
        XCTAssertEqual(store.state.current?.detached, false)
        XCTAssertEqual(store.state.current?.gone, false)

        await store.send(.closeTapped(id))
        await store.receive(\.closed)
        XCTAssertNil(store.state.selected)
        let left = try await daemon.client.terminals(nil)
        XCTAssertTrue(left.isEmpty)
    }

    func testATerminalClosedElsewhereIsGoneAndItsTabStillCloses() async throws {
        let daemon = InMemoryDaemon()
        let terminal = try await daemon.client.openTerminal(HostPlace(), .standard)
        let store = store(daemon, TerminalsFeature.State(place: HostPlace(), opensFirst: false))
        await store.send(.task)
        await store.receive(\.listed)
        try await daemon.client.closeTerminal(terminal)

        await store.send(.terminals(.element(id: terminal.id, action: .surfaceExited)))
        await store.receive(\.terminals[id: terminal.id].checked)
        XCTAssertEqual(store.state.current?.gone, true)
        XCTAssertEqual(store.state.current?.exited, true)
        XCTAssertEqual(store.state.current?.detached, false)

        // Reattach has nothing to attach to.
        await store.send(.terminals(.element(id: terminal.id, action: .reattachTapped)))
        XCTAssertEqual(store.state.current?.generation, 0)

        await store.send(.closeTapped(terminal.id))
        await store.receive(\.closed)
        XCTAssertTrue(store.state.tabs.isEmpty)
        XCTAssertNil(store.state.failure)
    }

    func testASurfaceThatEndsWhileTheShellRunsOffersReattachOnANewSurface() async throws {
        let daemon = InMemoryDaemon()
        let terminal = try await daemon.client.openTerminal(HostPlace(), .standard)
        let store = TestStore(initialState: TerminalFeature.State(terminal: terminal)) {
            TerminalFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.attach)
        let command = try XCTUnwrap(store.state.command)
        await store.send(.surfaceExited)
        await store.receive(\.checked)
        XCTAssertTrue(store.state.detached)
        XCTAssertFalse(store.state.exited)

        await store.send(.reattachTapped)
        XCTAssertFalse(store.state.detached)
        XCTAssertEqual(store.state.generation, 1)
        XCTAssertEqual(store.state.command, command)
    }

    func testAHostThatCantBeAskedLeavesReattachAndSaysWhy() async throws {
        let daemon = InMemoryDaemon()
        let terminal = try await daemon.client.openTerminal(HostPlace(), .standard)
        let store = TestStore(initialState: TerminalFeature.State(terminal: terminal)) {
            TerminalFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.terminals = { _ in throw DaemonError.unexpected("no answer") }
        }
        store.exhaustivity = .off

        await store.send(.surfaceExited)
        await store.receive(\.failed)
        XCTAssertTrue(store.state.detached)
        XCTAssertFalse(store.state.exited)
        XCTAssertNotNil(store.state.failure)

        await store.send(.reattachTapped)
        XCTAssertNil(store.state.failure)
        XCTAssertEqual(store.state.generation, 1)
    }

    func testClosingATerminalWhoseShellRunsAsksFirstAndEndsItOnce() async throws {
        let daemon = InMemoryDaemon()
        let terminal = try await daemon.client.openTerminal(HostPlace(), .standard)
        let closes = LockIsolated(0)
        // The host's answer waits, so a second press comes before it.
        let (answered, answer) = AsyncStream<Void>.makeStream()
        let store = store(daemon, TerminalsFeature.State(place: HostPlace(), opensFirst: false)) {
            let close = $0.daemon.closeTerminal
            $0.daemon.closeTerminal = { terminal in
                closes.withValue { $0 += 1 }
                for await _ in answered { break }
                try await close(terminal)
            }
        }
        await store.send(.task)
        await store.receive(\.listed)

        await store.send(.closeTapped(terminal.id))
        await Task.megaYield()
        XCTAssertNotNil(store.state.confirm)
        XCTAssertEqual(closes.value, 0, "nothing ends until the person says so")
        XCTAssertEqual(store.state.tabs.ids, [terminal.id])

        // Cancelled, the shell keeps running.
        await store.send(.confirm(.dismiss))
        await Task.megaYield()
        XCTAssertEqual(closes.value, 0)
        let running = try await daemon.client.terminals(nil)
        XCTAssertEqual(running.map(\.id), [terminal.id])

        await store.send(.closeTapped(terminal.id))
        await store.send(.confirm(.presented(.endShell(terminal.id))))
        XCTAssertTrue(store.state.closing.contains(terminal.id))
        // Pressed again while the host is asked: nothing more is sent or asked.
        await store.send(.closeTapped(terminal.id))
        await Task.megaYield()
        XCTAssertEqual(closes.value, 1)
        XCTAssertNil(store.state.confirm)

        answer.yield()
        await store.receive(\.closed)
        XCTAssertTrue(store.state.tabs.isEmpty)
        XCTAssertTrue(store.state.closing.isEmpty)
        let left = try await daemon.client.terminals(nil)
        XCTAssertTrue(left.isEmpty)
    }

    func testATerminalThatCouldNotBeClosedCanBeClosedAgain() async throws {
        let daemon = InMemoryDaemon()
        let terminal = try await daemon.client.openTerminal(HostPlace(), .standard)
        let fails = LockIsolated(true)
        let store = store(daemon, TerminalsFeature.State(place: HostPlace(), opensFirst: false)) {
            let close = $0.daemon.closeTerminal
            $0.daemon.closeTerminal = { terminal in
                if fails.value { throw DaemonError.unexpected("no answer") }
                try await close(terminal)
            }
        }
        await store.send(.task)
        await store.receive(\.listed)

        await store.send(.closeTapped(terminal.id))
        await store.send(.confirm(.presented(.endShell(terminal.id))))
        await store.receive(\.closeFailed)
        XCTAssertTrue(store.state.closing.isEmpty)
        XCTAssertNotNil(store.state.failure)
        XCTAssertEqual(store.state.tabs.ids, [terminal.id])

        fails.setValue(false)
        await store.send(.closeTapped(terminal.id))
        await store.send(.confirm(.presented(.endShell(terminal.id))))
        await store.receive(\.closed)
        XCTAssertTrue(store.state.tabs.isEmpty)
    }
}
