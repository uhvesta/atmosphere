import KhodrDaemonClient
import XCTest

final class InMemoryDaemonTests: XCTestCase {
    private func said(_ events: [SessionEvent]) -> [String] {
        events.compactMap { event in
            switch event.kind {
            case .userMessage(let text): "user: \(text)"
            case .assistantMessage(let text): "agent: \(text)"
            default: nil
            }
        }
    }

    func testAMessageIsAnswered() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "hello", .enqueue)
        let history = try await daemon.client.history(session.id)
        XCTAssertEqual(said(history), ["user: hello", "agent: You said: hello"])
    }

    func testAForkKeepsTheEventsBeforeItsEvent() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "one", .enqueue)
        try await daemon.client.send(session.id, "two", .enqueue)
        let history = try await daemon.client.history(session.id)
        let two = try XCTUnwrap(history.first { $0.kind == .userMessage("two") })

        let fork = try await daemon.client.fork(session.id, two.id, "fork")

        XCTAssertEqual(fork.parentID, session.id)
        XCTAssertEqual(fork.forkedBeforeEventID, two.id)
        let forked = try await daemon.client.history(fork.id)
        XCTAssertEqual(said(forked), ["user: one", "agent: You said: one"])
        XCTAssertEqual(said(daemon.history(session.id)).count, 4)
    }

    func testWatchingReplaysTheHistoryThenFollows() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "one", .enqueue)
        var events = await daemon.client.watch(session.id).makeAsyncIterator()
        try await daemon.client.send(session.id, "two", .enqueue)

        var seen: [SessionEvent] = []
        for _ in 0..<10 {
            if let event = try await events.next() { seen.append(event) }
        }
        XCTAssertEqual(said(seen), ["user: one", "agent: You said: one", "user: two", "agent: You said: two"])
    }

    func testAnUnknownSessionIsRefused() async {
        let daemon = InMemoryDaemon()
        do {
            _ = try await daemon.client.history("nope")
            XCTFail("Read an unknown session")
        } catch {
            XCTAssertEqual((error as? DaemonError).flatMap(code), .unknownSession)
        }
    }

    private func code(_ error: DaemonError) -> ErrorCode? {
        if case .refused(let code, _) = error { code } else { nil }
    }
}
