import ComposableArchitecture
import HumataDaemonClient
import XCTest

@testable import HumataSession

/// A history's chat worked out ahead, off the main thread, is the chat its
/// events make when they're taken in one at a time.
@MainActor
final class FoldedHistoryTests: XCTestCase {
    private func event(
        _ id: String, _ kind: SessionEvent.Kind, under call: String? = nil, at seconds: Double = 0
    ) -> SessionEvent {
        SessionEvent(id: id, time: Date(timeIntervalSince1970: seconds), kind: kind, parentCallID: call)
    }

    private func state() -> SessionFeature.State {
        SessionFeature.State(session: Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work"))
    }

    /// Turns of every kind: a write that asks first, a subagent with
    /// steps of its own, a rewind, a queued message, a compaction asked
    /// for, and a reply still being written with a request waiting.
    private var history: [SessionEvent] {
        let write = #"{"file_path":"notes.md","old_string":"one","new_string":"two"}"#
        let explore = Subagent(agentType: "Explore", description: "Find the parser", status: .running)
        let search = #"{"command":"rg parser"}"#
        return [
            event("e1", .userMessage("change the notes")),
            event("e2", .turnStarted),
            event("e3", .reasoning("Which file?")),
            event("e4", .reasoning(" The notes.")),
            event("e5", .toolStarted(callID: "w", tool: "Edit", argumentsJSON: write)),
            event("e6", .permissionRequested(requestID: "p1", kind: .write, summary: "Edit notes.md")),
            event("e7", .permissionCompleted(requestID: "p1", approved: true, automatic: false)),
            event("e8", .toolCompleted(callID: "w", success: true, output: "done")),
            event("e9", .toolStarted(callID: "a", tool: "Agent", argumentsJSON: "{}")),
            event("e10", .subagent(explore), under: "a"),
            event("e11", .toolStarted(callID: "a1", tool: "shell", argumentsJSON: search), under: "a"),
            event("d1", .assistantDelta("It's in"), under: "a", at: 1),
            event("e12", .toolCompleted(callID: "a1", success: true, output: "src/parser.rs"), under: "a", at: 1),
            event("e13", .assistantMessage("It's in src/parser.rs"), under: "a", at: 2),
            event("e14", .usage(model: "small", inputTokens: 100, outputTokens: 20, premiumRequests: 0), under: "a"),
            event("e15", .toolCompleted(callID: "a", success: true, output: "src/parser.rs"), at: 3),
            event("d2", .assistantDelta("Chan")),
            event("d3", .assistantDelta("ged.")),
            event("e16", .assistantMessage("Changed.")),
            event("e17", .messageQueued(queueID: "q1", text: "and the tests")),
            event("e18", .usage(model: "big", inputTokens: 10, outputTokens: 2, premiumRequests: 1)),
            event("e19", .turnEnded(aborted: false)),
            event("e20", .checkpoint),
            event("e21", .dequeued(queueID: "q1")),
            event("e22", .userMessage("and the tests")),
            event("e23", .turnStarted),
            event("e24", .failed("The model stopped answering.")),
            event("e25", .turnEnded(aborted: true)),
            event("e26", .rewound(text: "and the tests", files: false)),
            event("e27", .compacting(focus: "the notes")),
            event("e28", .turnStarted),
            event("e29", .compacted),
            event("e30", .turnEnded(aborted: false)),
            event("e31", .userMessage("go on")),
            event("e32", .turnStarted),
            event("e33", .permissionRequested(requestID: "p2", kind: .shell, summary: "make test")),
            event("d4", .assistantDelta("Runn")),
        ]
    }

    func testAHistoryFoldedAheadShowsTheSameChatAsOneTakenEventByEvent() {
        let history = history
        var ahead = state()
        ahead.receive(history, folded: FoldedHistory(history))
        var stepped = state()
        for event in history { stepped.receive(event) }

        XCTAssertFalse(ahead.items.isEmpty)
        XCTAssertEqual(ahead.items, stepped.items)
        XCTAssertEqual(ahead.events, stepped.events)
        XCTAssertEqual(ahead.subagents, stepped.subagents)
        ahead.shownSubagent = "a"
        stepped.shownSubagent = "a"
        XCTAssertFalse(ahead.subagentItems.isEmpty)
        XCTAssertEqual(ahead.subagentItems, stepped.subagentItems)
        // Everything else the chat's views read: whether it works or
        // waits, what it used, what is queued, the reply being written.
        XCTAssertEqual(ahead, stepped)
    }

    func testWhatComesAfterAHistoryFoldedAheadGoesOnTheEndOfItsChat() {
        let history = history
        let next = [event("e34", .assistantMessage("Running the tests.")), event("e35", .turnEnded(aborted: false))]
        var ahead = state()
        ahead.receive(history, folded: FoldedHistory(history))
        ahead.receive(next)
        var stepped = state()
        for event in history + next { stepped.receive(event) }

        XCTAssertEqual(ahead.items, stepped.items)
        XCTAssertEqual(ahead, stepped)
    }

    func testAHistoryThatRepeatsAnEventShowsItOnce() {
        let history = history
        // As a host sending part of it again would.
        let repeated = history + [history[0], history[18]]
        var ahead = state()
        ahead.receive(repeated, folded: FoldedHistory(repeated))
        var stepped = state()
        for event in history { stepped.receive(event) }

        XCTAssertEqual(ahead.items, stepped.items)
        XCTAssertEqual(ahead, stepped)
    }

    func testAHistoryFoldedAheadForAChatThatAlreadyHasSomeOfItAddsOnlyTheRest() {
        let history = history
        var ahead = state()
        ahead.receive(Array(history.prefix(8)))
        ahead.receive(history, folded: FoldedHistory(history))
        var stepped = state()
        for event in history { stepped.receive(event) }

        XCTAssertEqual(ahead.items, stepped.items)
        XCTAssertEqual(ahead, stepped)
    }

    func testAChatRestoredFromItsHistoryHasItsThumbnailsAndReadsNoneAgain() async {
        let picture = Data([1, 2, 3])
        var left = state()
        left.receive([
            event("e1", .userMessage("what is this", images: ["a.png"])), event("e2", .turnStarted),
            event("e3", .turnEnded(aborted: false)),
        ])
        left.historyLoaded = true
        left.thumbnails["a.png"] = picture
        let plain = left.history
        left.thumbnails["b.png"] = Data(repeating: 0, count: 4096)
        let kept = left.history
        XCTAssertNotNil(kept)
        XCTAssertGreaterThanOrEqual((kept?.bytes ?? 0) - (plain?.bytes ?? 0), 4096, "its thumbnails are counted")

        var again = state()
        if let kept { again.restore(kept) }
        XCTAssertEqual(again.thumbnails["a.png"], picture)

        // No daemon behind it: reading an image again would fail the test.
        let store = TestStore(initialState: again) {
            SessionFeature()
        }
        store.exhaustivity = .off
        await store.send(.thumbnailNeeded("a.png"))
        await store.finish()
        XCTAssertEqual(store.state.thumbnails["a.png"], picture)
    }
}
