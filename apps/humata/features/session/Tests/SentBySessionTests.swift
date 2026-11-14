import ComposableArchitecture
import HumataDaemonClient
import XCTest

@testable import HumataSession

/// A message another session's agent sent shows as from that session, not
/// as the person's.
@MainActor
final class SentBySessionTests: XCTestCase {
    private func event(_ id: String, _ kind: SessionEvent.Kind) -> SessionEvent {
        SessionEvent(id: id, time: Date(timeIntervalSince1970: 0), kind: kind)
    }

    private func state() -> SessionFeature.State {
        SessionFeature.State(session: Session(id: "s2", name: "Tests", backend: .scripted, workingDirectory: "/work"))
    }

    func testAMessageFromAnotherSessionsAgentShowsAsFromItAndThePersonsAsTheirs() throws {
        var state = state()
        let sent = AgentSender.headed(sessionID: "s1", name: "Lead", text: "Write the tests.")
        state.receive(event("e1", .userMessage(sent)))
        state.receive(event("e2", .turnStarted))
        state.receive(event("e3", .assistantMessage("On it.")))
        state.receive(event("e4", .turnEnded(aborted: false)))
        state.receive(event("e5", .userMessage("And the docs.")))

        let sender = try XCTUnwrap(AgentSender(sent))
        XCTAssertEqual(
            state.items,
            [
                .fromSession(id: "e1", sender),
                .reply(id: "e3", text: "On it."),
                .person(id: "e5", text: "And the docs."),
            ])
        guard case .fromSession(_, let shown)? = state.items.first else { return XCTFail("Not from a session") }
        XCTAssertEqual(shown.name, "Lead")
        XCTAssertEqual(shown.text, "Write the tests.")
    }

    func testAHistoryReadWholeShowsTheSameAsItsEventsOneAtATime() {
        let sent = AgentSender.headed(sessionID: "s1", name: "Lead", text: "Write the tests.")
        let history = [
            event("e1", .userMessage(sent)),
            event("e2", .turnStarted),
            event("e3", .assistantMessage("On it.")),
            event("e4", .turnEnded(aborted: false)),
        ]
        var whole = state()
        whole.receive(history)
        var each = state()
        for event in history { each.receive(event) }
        XCTAssertEqual(whole.items, each.items)
    }
}
