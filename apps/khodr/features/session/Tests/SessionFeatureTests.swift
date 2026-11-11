import ComposableArchitecture
import KhodrDaemonClient
import KhodrSession
import XCTest

@MainActor
final class SessionFeatureTests: XCTestCase {
    private func started(_ daemon: InMemoryDaemon) async throws -> Session {
        try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work", name: "first"))
    }

    private func said(_ events: IdentifiedArrayOf<SessionEvent>) -> [String] {
        events.compactMap { event in
            switch event.kind {
            case .userMessage(let text): "user: \(text)"
            case .assistantMessage(let text): "agent: \(text)"
            default: nil
            }
        }
    }

    func testWatchingShowsTheHistoryThenEachNewEvent() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        try await daemon.client.send(session.id, "earlier", .enqueue)
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        let watching = await store.send(.task)
        await store.receive(\.received, timeout: .seconds(1))
        await store.skipReceivedActions()
        XCTAssertEqual(said(store.state.events), ["user: earlier", "agent: You said: earlier"])

        await store.send(\.binding.draft, "  hello  ")
        await store.send(.sendTapped(.enqueue)) {
            $0.draft = ""
        }
        await store.skipReceivedActions()
        XCTAssertEqual(
            said(store.state.events),
            ["user: earlier", "agent: You said: earlier", "user: hello", "agent: You said: hello"])
        XCTAssertFalse(store.state.isWorking)
        await watching.cancel()
    }

    func testABlankMessageIsNotSent() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = SessionFeature.State(session: session)
        state.draft = "   "
        let store = TestStore(initialState: state) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.send = { _, _, _ in XCTFail("sent a blank message") }
        }
        await store.send(.sendTapped(.enqueue))
    }

    func testForkingBeforeAnEventOpensTheFork() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        try await daemon.client.send(session.id, "one", .enqueue)
        try await daemon.client.send(session.id, "two", .enqueue)
        let two = try XCTUnwrap(daemon.history(session.id).first { $0.kind == .userMessage("two") })
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }

        await store.send(.forkTapped(before: two.id))
        let fork = try XCTUnwrap(daemon.sessions.last)
        XCTAssertEqual(fork.parentID, session.id)
        XCTAssertEqual(fork.forkedBeforeEventID, two.id)
        await store.receive(\.forked, fork)
        await store.receive(\.delegate.openSession, fork)
        XCTAssertEqual(daemon.history(fork.id), Array(daemon.history(session.id).prefix { $0.id != two.id }))
    }

    func testARefusedRequestShowsWhatToDo() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let refusal = DaemonError.refused(.busy, message: "Busy; wait for the turn to end.")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.abort = { _ in throw refusal }
        }
        store.exhaustivity = .off

        await store.send(.abortTapped)
        await store.receive(\.failed)
        XCTAssertEqual(store.state.alert?.message, TextState(refusal.description))
    }

    func testALostConnectionIsShownAndWatchingAgainResumes() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.watch = { _ in AsyncThrowingStream { $0.finish(throwing: DaemonError.disconnected) } }
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.watchEnded)
        XCTAssertTrue(store.state.disconnected)
        XCTAssertNotNil(store.state.alert)

        await store.send(.task) {
            $0.disconnected = false
        }
        await store.receive(\.watchEnded)
    }

    private func store() -> TestStoreOf<SessionFeature> {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) { SessionFeature() }
        store.exhaustivity = .off
        return store
    }

    private func event(_ id: String, _ kind: SessionEvent.Kind) -> SessionEvent {
        SessionEvent(id: id, time: Date(timeIntervalSince1970: 0), kind: kind)
    }

    func testAReplyStreamsUntilItsMessageArrives() async {
        let store = store()
        await store.send(.received(event("e1", .turnStarted)))
        await store.send(.received(event("e2", .assistantDelta("hel"))))
        await store.send(.received(event("e3", .assistantDelta("lo"))))
        XCTAssertEqual(store.state.streaming, "hello")
        XCTAssertTrue(store.state.isWorking)
        await store.send(.received(event("e4", .assistantMessage("hello"))))
        await store.send(.received(event("e5", .turnEnded(aborted: false))))
        XCTAssertEqual(store.state.streaming, "")
        XCTAssertFalse(store.state.isWorking)
        XCTAssertEqual(store.state.events.ids, ["e1", "e4", "e5"])
    }

    func testOnlyUnansweredPermissionsArePending() async {
        let store = store()
        await store.send(.received(event("e1", .permissionRequested(requestID: "p1", kind: .shell, summary: "ls"))))
        await store.send(.received(event("e2", .permissionRequested(requestID: "p2", kind: .write, summary: "a"))))
        await store.send(
            .received(event("e3", .permissionCompleted(requestID: "p1", approved: true, automatic: false))))
        XCTAssertEqual(store.state.pendingPermissions.map(\.id), ["e2"])
    }
}
