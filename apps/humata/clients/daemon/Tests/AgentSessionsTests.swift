import HumataDaemonClient
import XCTest

/// Messages one session's agent sends another, and what an agent asks to do
/// to other sessions.
final class AgentSessionsTests: XCTestCase {
    func testAMessageFromAnAgentNamesItsSenderAndKeepsItsText() throws {
        let text = "Run the tests.\n\nThen say what failed."
        let sent = AgentSender.headed(sessionID: "s1", name: "Build the app", text: text)
        let sender = try XCTUnwrap(AgentSender(sent))
        XCTAssertEqual(sender.sessionID, "s1")
        XCTAssertEqual(sender.name, "Build the app")
        XCTAssertEqual(sender.text, text)
    }

    func testAMessageFromThePersonHasNoSender() {
        XCTAssertNil(AgentSender("Run the tests."))
        XCTAssertNil(AgentSender(""))
        // Quoting such a message later in one's own doesn't make it one.
        let quoted = "See: " + AgentSender.headed(sessionID: "s1", name: "Build", text: "hi")
        XCTAssertNil(AgentSender(quoted))
    }

    func testANameThatWouldEndTheLineEarlyStillReadsBack() throws {
        let sent = AgentSender.headed(sessionID: "s1", name: "The \"big\"\none", text: "hi")
        let sender = try XCTUnwrap(AgentSender(sent))
        XCTAssertEqual(sender.sessionID, "s1")
        XCTAssertEqual(sender.text, "hi")
    }

    func testAllowingASessionAnAgentAskedForStartsItUnderTheAgentsWithItsMessage() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        let parent = try await daemon.client.start(
            StartRequest(backend: .scripted, name: "Lead", projectID: project.id, itemID: task.id))
        let request = daemon.addRequest(parent.id, .startSession(name: "Tests"), "Write the tests.")

        // It waits as what an agent asks, and nothing starts meanwhile.
        let waiting = try await daemon.client.inbox()
        XCTAssertEqual(waiting.entries.compactMap(\.request), [request])
        let before = try await daemon.client.sessions(nil)
        XCTAssertEqual(before.map(\.id), [parent.id])

        try await daemon.client.answerRequest(request.id, .approve)
        let sessions = try await daemon.client.sessions(nil)
        let child = try XCTUnwrap(sessions.first { $0.startedBy == parent.id })
        XCTAssertEqual(child.name, "Tests")
        XCTAssertEqual(child.projectID, project.id)
        XCTAssertEqual(child.itemID, task.id)
        XCTAssertEqual(child.nestsUnder, parent.id)
        // Not a fork: it has none of its parent's history.
        XCTAssertNil(child.parentID)

        let first = try await daemon.client.history(child.id).compactMap { event -> AgentSender? in
            guard case .userMessage(let text, _) = event.kind else { return nil }
            return AgentSender(text)
        }
        XCTAssertEqual(first.map(\.sessionID), [parent.id])
        XCTAssertEqual(first.map(\.text), ["Write the tests."])
        let after = try await daemon.client.inbox()
        XCTAssertFalse(after.entries.contains { $0.request != nil })
    }

    func testDecliningWhatAnAgentAskedForStartsAndSendsNothing() async throws {
        let daemon = InMemoryDaemon()
        let parent = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        let other = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        let start = daemon.addRequest(parent.id, .startSession(name: "Tests"), "Write the tests.")
        let send = daemon.addRequest(parent.id, .sendMessage(to: other.id, name: other.name), "Stop.")

        try await daemon.client.answerRequest(start.id, .dismiss)
        try await daemon.client.answerRequest(send.id, .dismiss)
        let sessions = try await daemon.client.sessions(nil)
        XCTAssertEqual(sessions.map(\.id), [parent.id, other.id])
        let history = try await daemon.client.history(other.id)
        XCTAssertTrue(history.isEmpty)
    }

    func testAllowingAMessageAnAgentAskedToSendDeliversItAsTheAgents() async throws {
        let daemon = InMemoryDaemon()
        let parent = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        let other = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        let send = daemon.addRequest(parent.id, .sendMessage(to: other.id, name: other.name), "Stop.")

        try await daemon.client.answerRequest(send.id, .approve)
        let received = try await daemon.client.history(other.id).compactMap { event -> AgentSender? in
            guard case .userMessage(let text, _) = event.kind else { return nil }
            return AgentSender(text)
        }
        XCTAssertEqual(received.map(\.sessionID), [parent.id])
        XCTAssertEqual(received.map(\.text), ["Stop."])
    }
}
