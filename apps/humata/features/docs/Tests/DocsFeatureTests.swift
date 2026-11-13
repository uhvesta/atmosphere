import ComposableArchitecture
import HumataDaemonClient
import HumataDocs
import XCTest

@MainActor
final class DocsFeatureTests: XCTestCase {
    private let plan = "# Plan\n\nShip the reader.\nThen the chat.\n"

    /// A chat in no folder whose agent wrote a plan and some notes.
    private func chat() async throws -> (InMemoryDaemon, Session) {
        let plan = plan
        let daemon = InMemoryDaemon { text in
            guard text.hasPrefix("Write") else { return [.say("On it.")] }
            return [
                .write(path: "notes.txt", contents: "one\ntwo\nthree\n"),
                .write(path: "plan.md", contents: plan),
                .say("Done."),
            ]
        }
        let session = try await daemon.client.start(StartRequest(backend: .scripted, approvalMode: .allowEverything))
        try await daemon.client.send(session.id, "Write the plan", .enqueue)
        return (daemon, session)
    }

    private func store(_ daemon: InMemoryDaemon, _ session: Session) -> TestStoreOf<DocsFeature> {
        let store = TestStore(initialState: DocsFeature.State(sessionID: session.id)) {
            DocsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return store
    }

    func testTheFolderOpensOnItsFirstMarkdownFile() async throws {
        let (daemon, session) = try await chat()
        let store = store(daemon, session)
        await store.send(.refresh)
        await store.receive(\.listed) {
            $0.selected = "plan.md"
        }
        XCTAssertEqual(store.state.files.map(\.path), ["notes.txt", "plan.md"])
        await store.receive(\.read) {
            $0.show(FileContents(path: "plan.md", text: self.plan))
        }
        XCTAssertEqual(store.state.lines?.first, "# Plan")
    }

    func testCommentsAreDraftsUntilSentAsOneMessage() async throws {
        let (daemon, session) = try await chat()
        let store = store(daemon, session)
        await store.send(.refresh)
        await store.receive(\.read)

        await store.send(.blockTapped(startLine: 3, endLine: 4, quote: "Ship the reader.\nThen the chat."))
        await store.send(.binding(.set(\.comment, "Reader first, chat after.")))
        await store.send(.commentSaved)
        await store.send(.fileSelected("notes.txt"))
        await store.receive(\.read)
        await store.send(.lineTapped(1, extending: false))
        await store.send(.lineTapped(2, extending: true)) {
            $0.picked = DocsFeature.Picked(path: "notes.txt", startLine: 1, endLine: 2, quote: "one\ntwo")
        }
        await store.send(.binding(.set(\.comment, "Merge these.")))
        await store.send(.commentSaved)
        XCTAssertEqual(store.state.drafts.count, 2)
        XCTAssertEqual(daemon.history(session.id).filter(\.isUserMessage).count, 1, "Drafts aren't sent")

        await store.send(.sendTapped)
        await store.receive(\.sent) {
            $0.drafts = []
        }
        let sent = daemon.history(session.id).compactMap { event -> String? in
            if case .userMessage(let text, _) = event.kind { text } else { nil }
        }
        let message = try XCTUnwrap(sent.last)
        XCTAssertEqual(sent.count, 2)
        for part in ["plan.md", "> Ship the reader.", "Reader first", "notes.txt", "> two", "Merge these."] {
            XCTAssertTrue(message.contains(part), part)
        }
    }

    func testARemovedDraftIsNotSent() async throws {
        let (daemon, session) = try await chat()
        let store = store(daemon, session)
        await store.send(.refresh)
        await store.receive(\.read)
        await store.send(.lineTapped(1, extending: false))
        await store.send(.binding(.set(\.comment, "Rename it.")))
        await store.send(.commentSaved)
        let draft = try XCTUnwrap(store.state.drafts.first)
        await store.send(.draftRemoved(draft.id)) {
            $0.drafts = []
        }
        await store.send(.sendTapped)
        XCTAssertFalse(store.state.sending)
    }

    func testAnEmptyCommentIsNotSaved() async throws {
        let (daemon, session) = try await chat()
        let store = store(daemon, session)
        await store.send(.refresh)
        await store.receive(\.read)
        await store.send(.lineTapped(1, extending: false))
        await store.send(.binding(.set(\.comment, "   ")))
        await store.send(.commentSaved)
        XCTAssertEqual(store.state.drafts, [])
        XCTAssertNotNil(store.state.picked)
    }

    func testAFileThatCouldntBeReadSaysSoUntilItIs() async throws {
        let (daemon, session) = try await chat()
        let reachable = LockIsolated(false)
        let store = TestStore(initialState: DocsFeature.State(sessionID: session.id)) {
            DocsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.readFile = { session, path in
                guard reachable.value else { throw DaemonError.disconnected }
                return try await daemon.client.readFile(session, path)
            }
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off

        await store.send(.refresh)
        await store.receive(\.listed)
        await store.receive(\.failed)
        XCTAssertNotNil(store.state.readFailure)
        XCTAssertNil(store.state.alert)

        reachable.setValue(true)
        await store.send(.refresh)
        await store.receive(\.read)
        XCTAssertNil(store.state.readFailure)
        XCTAssertEqual(store.state.lines?.first, "# Plan")
    }

    func testAFailedSendKeepsTheDrafts() async throws {
        let (daemon, session) = try await chat()
        let store = store(daemon, session)
        await store.send(.refresh)
        await store.receive(\.read)
        await store.send(.lineTapped(1, extending: false))
        await store.send(.binding(.set(\.comment, "Rename it.")))
        await store.send(.commentSaved)
        _ = try await daemon.client.update(session.id, SessionUpdate(archived: true))
        await store.send(.sendTapped)
        await store.receive(\.failed)
        XCTAssertEqual(store.state.drafts.count, 1)
        XCTAssertFalse(store.state.sending)
    }
}

extension SessionEvent {
    fileprivate var isUserMessage: Bool {
        if case .userMessage = kind { true } else { false }
    }
}
