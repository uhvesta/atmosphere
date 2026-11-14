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

    func testANoticeOfHumatasShowsAsANoticeAndNotAsThePersonsMessage() {
        var state = state()
        state.receive(event("e1", .userMessage("Start the tests session.")))
        state.receive(event("e2", .turnStarted))
        state.receive(event("e3", .turnEnded(aborted: false)))
        state.receive(event("e4", .userMessage(HumataNotice.headed("The person allowed it: Tests has started."))))
        state.receive(event("e5", .turnStarted))

        XCTAssertEqual(
            state.items,
            [
                .person(id: "e1", text: "Start the tests session."),
                .notice(id: "e4", text: "The person allowed it: Tests has started."),
            ])
        XCTAssertFalse(state.lastMessageOwn, "there's no message of the person's to stop and edit")
    }

    func testOnlyThePersonsOwnMessageIsStoppedAndEditedOrOpenedToEdit() async {
        let stopped = LockIsolated(0)
        let store = TestStore(initialState: state()) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.abort = { _ in stopped.withValue { $0 += 1 } }
        }
        store.exhaustivity = .off
        let sent = AgentSender.headed(sessionID: "s1", name: "Lead", text: "Write the tests.")
        await store.send(.received(event("e1", .userMessage(sent))))
        await store.send(.received(event("e2", .turnStarted)))

        await store.send(.stopAndEditTapped)
        await store.send(.editTapped("e1"))
        await store.finish()
        XCTAssertEqual(stopped.value, 0)
        XCTAssertEqual(store.state.draft, "")
        XCTAssertNil(store.state.editing)
    }

    func testRewindListsAnAgentsMessageByWhatItSaysAndANoticeAsHumatas() {
        let sent = AgentSender.headed(sessionID: "s1", name: "Lead", text: "Write the tests.\nAll of them.")
        let points = RewindPoint.points(in: [
            event("e1", .userMessage(sent)),
            event("e2", .userMessage(HumataNotice.headed("The person declined it."))),
            event("e3", .userMessage("And the docs.")),
        ])
        XCTAssertEqual(points.map(\.id), ["e3", "e2", "e1"])
        XCTAssertEqual(points.map(\.firstLine), ["And the docs.", "The person declined it.", "Write the tests."])
        XCTAssertEqual(points.map(\.sender), [.person, .humata, .session("Lead")])
        XCTAssertEqual(RewindPoint.matching("write", in: points).map(\.id), ["e1"])
        // The line naming the sender isn't searched.
        XCTAssertEqual(RewindPoint.matching("message from", in: points), [])
    }

    func testRewindingToAMessageThatIsntThePersonsLeavesTheBoxEmpty() async {
        let rewound = LockIsolated<[String]>([])
        let store = TestStore(initialState: state()) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.rewind = { _, event, _ in rewound.withValue { $0.append(event) } }
        }
        store.exhaustivity = .off
        let sent = AgentSender.headed(sessionID: "s1", name: "Lead", text: "Write the tests.")
        await store.send(.received(event("e1", .userMessage(sent))))

        await store.send(.rewindTapped("e1"))
        XCTAssertNotNil(store.state.rewindDialog)
        await store.send(.rewindDialog(.presented(.chat("e1"))))
        await store.receive(\.rewound)
        XCTAssertEqual(rewound.value, ["e1"])
        XCTAssertEqual(store.state.draft, "")
        XCTAssertEqual(store.state.pastes, [])
    }
}
