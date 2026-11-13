import ComposableArchitecture
import HumataDaemonClient
import XCTest

@testable import HumataSession

/// A long chat opened after a restart: its whole history comes at once and
/// shows quickly.
@MainActor
final class LongChatTests: XCTestCase {
    /// Turns in the history, each of ten events.
    private static let turns = 500

    /// A history of `turns` turns, each a message (every fifth a long
    /// paste), thoughts, a command with long output, a reply and its usage.
    static func history() -> [SessionEvent] {
        let paste = String(repeating: "A pasted log line that goes on for a while.\n", count: 700)
        let thoughts = String(repeating: "Check the fixture parses, then the next one.\n", count: 70)
        let output = String(repeating: "row,ok,2026-11-13,a value long enough to wrap\n", count: 1_100)
        var events: [SessionEvent] = []
        func add(_ kind: SessionEvent.Kind) {
            events.append(SessionEvent(id: "e\(events.count)", time: Date(timeIntervalSince1970: 0), kind: kind))
        }
        for turn in 0..<turns {
            add(.userMessage(turn % 5 == 0 ? "Look at this:\n\(paste)" : "Check fixture \(turn)"))
            add(.turnStarted)
            add(.reasoning(thoughts))
            add(.toolStarted(callID: "c\(turn)", tool: "shell", argumentsJSON: #"{"command":"cat fixtures.csv"}"#))
            add(.permissionRequested(requestID: "p\(turn)", kind: .shell, summary: "cat fixtures.csv"))
            add(.permissionCompleted(requestID: "p\(turn)", approved: true, automatic: true))
            add(.toolCompleted(callID: "c\(turn)", success: true, output: output))
            add(.assistantMessage("Fixture \(turn) parses; **all rows** are fine."))
            add(.usage(model: "a", inputTokens: 100, outputTokens: 10, premiumRequests: 1))
            add(.turnEnded(aborted: false))
        }
        return events
    }

    func testALongChatOpensQuicklyAfterARestart() async throws {
        let history = Self.history()
        XCTAssertEqual(history.count, 5_000)
        let session = Session(id: "s1", name: "long", backend: .scripted, workingDirectory: "/work")
        // A store as the app has, not a TestStore, which checks the state
        // after each action and would time itself.
        let store = Store(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.watch = { _ in
                AsyncThrowingStream { continuation in
                    for event in history { continuation.yield(event) }
                }
            }
            $0.daemon.markSeen = { _, _ in }
            $0.appActivity = AppActivity(isActive: { true }, activations: { AsyncStream { _ in } }, setBadge: { _ in })
        }

        let clock = ContinuousClock()
        let start = clock.now
        let task = store.send(.task)
        while store.events.count < history.count, clock.now - start < .seconds(60) {
            try await Task.sleep(for: .milliseconds(5))
        }
        let opened = clock.now - start
        task.cancel()

        XCTAssertTrue(store.state.historyLoaded)
        XCTAssertEqual(store.state.events.count, history.count)
        // Each turn shows its message, thoughts, command, reply and cost.
        XCTAssertEqual(store.state.items.count, Self.turns * 5)
        XCTAssertFalse(store.state.isWorking)
        XCTAssertTrue(store.state.pendingPermissions.isEmpty)
        XCTAssertEqual(store.state.turnsEnded, Self.turns)
        XCTAssertEqual(store.state.usage?.inputTokens, UInt64(Self.turns * 100))
        let tall = store.state.items.filter { item in
            if case .tool(let call) = item { call.preview?.isTall == true } else { false }
        }
        XCTAssertEqual(tall.count, Self.turns)
        // Generous, so a busy CI machine doesn't fail it; a chat worked out
        // again for each event takes minutes.
        XCTAssertLessThan(opened, .seconds(10), "opening took \(opened)")
        print("A chat of \(history.count) events opened in \(opened)")
    }

    func testWorkingOutALongChatIsQuick() {
        let history = Self.history()
        let session = Session(id: "s1", name: "long", backend: .scripted, workingDirectory: "/work")
        measure {
            var state = SessionFeature.State(session: session)
            state.receive(history)
            XCTAssertEqual(state.items.count, Self.turns * 5)
        }
    }
}
