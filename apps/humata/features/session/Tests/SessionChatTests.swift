import ComposableArchitecture
import HumataDaemonClient
import XCTest

@testable import HumataSession

/// What a session's events add up to, kept as they come, whichever way
/// they come.
@MainActor
final class SessionChatTests: XCTestCase {
    /// What the session shows of its events besides the chat.
    private struct Summary: Equatable {
        var working: Bool
        var lastMessage: String?
        var lastTurnFailed: Bool
        var retryable: String?
        var usage: Usage?
        var pending: [String]
        var queued: [String]
        var dropped: [String]
        var turnsStarted: Int
        var turnsEnded: Int
        var proposing: Set<String>
        var items: [ChatItem]

        init(_ state: SessionFeature.State) {
            working = state.isWorking
            lastMessage = state.lastMessage
            lastTurnFailed = state.lastTurnFailed
            retryable = state.retryableFailure
            usage = state.usage
            pending = state.pendingPermissions.map(\.id)
            queued = state.queued.map(\.text)
            dropped = state.dropped.map(\.text)
            turnsStarted = state.turnsStarted
            turnsEnded = state.turnsEnded
            proposing = state.proposingCalls
            items = state.items
        }
    }

    private func event(_ id: String, _ kind: SessionEvent.Kind) -> SessionEvent {
        SessionEvent(id: id, time: Date(timeIntervalSince1970: 0), kind: kind)
    }

    private func state() -> SessionFeature.State {
        SessionFeature.State(session: Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work"))
    }

    /// Two turns: one that fails, with a message dropped from the queue,
    /// and one under way, waiting on a permission, that proposes a task.
    private var history: [SessionEvent] {
        [
            event("e1", .userMessage("write the notes")),
            event("e2", .turnStarted),
            event("e3", .messageQueued(queueID: "q1", text: "then the changelog")),
            event("e4", .usage(model: "a", inputTokens: 10, outputTokens: 2, premiumRequests: 1)),
            event("e5", .failed("The model went away")),
            event("e6", .turnEnded(aborted: false)),
            event("e7", .dequeued(queueID: "q1", dropped: true)),
            event("e8", .messageQueued(queueID: "q2", text: "and tag it")),
            event("e9", .turnStarted),
            event("e10", .dequeued(queueID: "q2")),
            event("e11", .toolStarted(callID: "c1", tool: "mcp__humata__propose_task", argumentsJSON: "{}")),
            event("e12", .permissionRequested(requestID: "p1", kind: .shell, summary: "ls")),
            event("e13", .permissionRequested(requestID: "p2", kind: .shell, summary: "pwd")),
            event("e14", .permissionCompleted(requestID: "p1", approved: true, automatic: false)),
            event("e15", .usage(model: "b", inputTokens: 5, outputTokens: 3, premiumRequests: 0.5)),
        ]
    }

    func testAHistoryAddsUpToWhatItsTurnsLeft() {
        var state = state()
        state.receive(history)
        let summary = Summary(state)
        XCTAssertTrue(summary.working)
        XCTAssertEqual(summary.lastMessage, "write the notes")
        XCTAssertFalse(summary.lastTurnFailed)
        XCTAssertNil(summary.retryable)
        XCTAssertEqual(summary.usage, Usage(model: "b", inputTokens: 15, outputTokens: 5, premiumRequests: 1.5))
        XCTAssertEqual(summary.pending, ["e13"])
        XCTAssertEqual(summary.queued, [])
        XCTAssertEqual(summary.dropped, ["then the changelog"])
        XCTAssertEqual(summary.turnsStarted, 2)
        XCTAssertEqual(summary.turnsEnded, 1)
        XCTAssertEqual(summary.proposing, ["c1"])
    }

    func testAFailedTurnCanBeTriedAgainUntilTheNextStarts() {
        var state = state()
        state.receive(Array(history.prefix(6)))
        XCTAssertTrue(state.lastTurnFailed)
        XCTAssertEqual(state.retryableFailure, "e5")
        XCTAssertFalse(state.isWorking)
        state.receive(event("e9", .turnStarted))
        XCTAssertFalse(state.lastTurnFailed)
        XCTAssertNil(state.retryableFailure)
    }

    func testEventsOneAtATimeAddUpAsTheWholeHistoryDoes() {
        var whole = state()
        whole.receive(history)
        var each = state()
        for (index, event) in history.enumerated() {
            each.receive(event)
            var upTo = state()
            upTo.events = IdentifiedArray(uniqueElements: history.prefix(index + 1))
            XCTAssertEqual(Summary(each), Summary(upTo), "after \(event.id)")
        }
        XCTAssertEqual(Summary(each), Summary(whole))
    }

    func testAChangedEventAddsUpAsIfItCameThatWay() {
        var changed = state()
        changed.receive(history)
        changed.receive(event("e12", .permissionRequested(requestID: "p1", kind: .shell, summary: "ls -la")))
        var history = history
        history[11] = event("e12", .permissionRequested(requestID: "p1", kind: .shell, summary: "ls -la"))
        var fresh = state()
        fresh.receive(history)
        XCTAssertEqual(Summary(changed), Summary(fresh))
    }
}
