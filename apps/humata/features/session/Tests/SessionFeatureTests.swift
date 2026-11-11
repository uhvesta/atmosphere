import ComposableArchitecture
import HumataDaemonClient
import HumataSession
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

    func testTheReviewIsReadWhenPickedAndAgainWhenATurnEnds() async throws {
        let daemon = InMemoryDaemon { text in [.write(path: "\(text).txt", contents: text)] }
        let project = try await daemon.client.createProject("Humata", [Repository(path: "/code/humata")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, approvalMode: .allowEverything, projectID: project.id))
        try await daemon.client.send(session.id, "one", .enqueue)
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        let watching = await store.send(.task)
        await store.send(.tabPicked(.review))
        await store.receive(\.review.loaded)
        XCTAssertEqual(store.state.review.files.map(\.path), ["one.txt"])

        try await daemon.client.send(session.id, "two", .enqueue)
        await store.receive(\.review.loaded, timeout: .seconds(1))
        XCTAssertEqual(store.state.review.files.map(\.path), ["one.txt", "two.txt"])
        XCTAssertEqual(store.state.review.turns, 2)
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

        await store.send(.forkTapped(before: two.id, chatOnly: false))
        let fork = try XCTUnwrap(daemon.sessions.last)
        XCTAssertEqual(fork.parentID, session.id)
        XCTAssertEqual(fork.forkedBeforeEventID, two.id)
        await store.receive(\.forked, fork)
        await store.receive(\.delegate.openSession, fork)
        XCTAssertEqual(daemon.history(fork.id), Array(daemon.history(session.id).prefix { $0.id != two.id }))
    }

    func testEditingAnEarlierMessageSendsItToAForkAndLeavesTheSessionAsItWas() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        try await daemon.client.send(session.id, "one", .enqueue)
        try await daemon.client.send(session.id, "two", .enqueue)
        let before = daemon.history(session.id)
        let two = try XCTUnwrap(before.first { $0.kind == .userMessage("two") })
        var state = SessionFeature.State(session: session)
        state.events = IdentifiedArray(uniqueElements: before)
        let store = TestStore(initialState: state) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }

        await store.send(.editTapped(two.id)) {
            $0.editing = two.id
            $0.edited = "two"
        }
        await store.send(.binding(.set(\.edited, "three"))) {
            $0.edited = "three"
        }
        await store.send(.editSent) {
            $0.editing = nil
            $0.edited = ""
        }
        // The edit runs as the fork's first turn, so it has moved on since.
        await store.receive(\.forked)
        await store.receive(\.delegate.openSession)
        let fork = try XCTUnwrap(daemon.sessions.last)
        XCTAssertEqual(fork.forkedBeforeEventID, two.id)
        let said = daemon.history(fork.id).compactMap { event -> String? in
            if case .userMessage(let text) = event.kind { text } else { nil }
        }
        XCTAssertEqual(said, ["one", "three"])
        XCTAssertEqual(daemon.history(session.id), before)
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

    private let edit: @Sendable (String) -> [InMemoryDaemon.Step] = { _ in
        [
            .think("The notes go in the repository."), .write(path: "/work/NOTES.md", contents: "Ship M1."),
            .run(command: "git status", output: "clean"), .say("Done."),
        ]
    }

    /// A store watching a session of `daemon`, after `.task` has replayed
    /// what is already there.
    private func watching(_ daemon: InMemoryDaemon, _ session: Session) async -> (
        TestStoreOf<SessionFeature>, TestStoreTask
    ) {
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        let task = await store.send(.task)
        return (store, task)
    }

    private func until(_ store: TestStoreOf<SessionFeature>, _ done: (SessionFeature.State) -> Bool) async {
        for _ in 0..<100 where !done(store.state) {
            await store.skipReceivedActions(strict: false)
            await Task.yield()
        }
        XCTAssertTrue(done(store.state), "never reached")
    }

    func testAnEditWaitsForApprovalThenShowsWhatItDid() async throws {
        let daemon = InMemoryDaemon(script: edit)
        let session = try await started(daemon)
        let (store, task) = await watching(daemon, session)

        await store.send(\.binding.draft, "write the notes")
        await store.send(.sendTapped(.enqueue))
        await until(store) { $0.isWaiting }
        XCTAssertEqual(daemon.files(of: session.id), [:])
        guard case .tool(let write)? = store.state.items.last else { return XCTFail("no tool call") }
        XCTAssertEqual(write.subject, "/work/NOTES.md")
        XCTAssertNil(write.permission?.answer)
        let request = try XCTUnwrap(write.permission?.requestID)

        await store.send(.answerPermission(requestID: request, approved: true))
        await until(store) { state in
            state.isWaiting && state.items.filter { if case .tool = $0 { true } else { false } }.count == 2
        }
        XCTAssertEqual(daemon.files(of: session.id), ["/work/NOTES.md": "Ship M1."])
        let calls = store.state.items.compactMap { item -> ToolCall? in
            if case .tool(let call) = item { call } else { nil }
        }
        XCTAssertEqual(calls.map(\.subject), ["/work/NOTES.md", "git status"])
        XCTAssertEqual(calls.first?.succeeded, true)
        XCTAssertEqual(calls.first?.permission?.answer, .approved)
        XCTAssertTrue(
            store.state.items.contains(
                .thinking(
                    id: try XCTUnwrap(store.state.items.dropFirst().first?.id),
                    text: "The notes go in the repository.")))

        let command = try XCTUnwrap(calls.last?.permission?.requestID)
        await store.send(.answerPermission(requestID: command, approved: false))
        await until(store) { !$0.isWorking }
        guard case .reply(_, "Done.")? = store.state.items.last else { return XCTFail("no reply") }
        let denied = store.state.items.compactMap { item -> ToolCall? in
            if case .tool(let call) = item { call } else { nil }
        }.last
        XCTAssertEqual(denied?.succeeded, false)
        XCTAssertEqual(denied?.permission?.answer, .denied)
        await task.cancel()
    }

    func testAllowingEditsLetsTheWaitingWriteGoAhead() async throws {
        let daemon = InMemoryDaemon(script: edit)
        let session = try await started(daemon)
        let (store, task) = await watching(daemon, session)
        try await daemon.client.send(session.id, "write the notes", .enqueue)
        await until(store) { $0.isWaiting }

        await store.send(.approvalModePicked(.allowEdits)) {
            $0.session.approvalMode = .allowEdits
        }
        await until(store) { state in
            state.items.contains { item in
                if case .tool(let call) = item { call.permission?.answer == .allowedByMode } else { false }
            }
        }
        XCTAssertEqual(daemon.files(of: session.id), ["/work/NOTES.md": "Ship M1."])
        XCTAssertEqual(daemon.sessions.first?.approvalMode, .allowEdits)
        // The command still asks.
        XCTAssertTrue(store.state.isWaiting)
        await task.cancel()
    }

    func testARefusedModeKeepsTheOldOne() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.setApprovalMode = { _, _ in throw DaemonError.disconnected }
        }
        await store.send(.approvalModePicked(.allowEverything)) {
            $0.session.approvalMode = .allowEverything
        }
        await store.receive(\.approvalModeRefused) {
            $0.session.approvalMode = .askEveryTime
            $0.alert = AlertState {
                TextState("Something went wrong")
            } message: {
                TextState(DaemonError.disconnected.description)
            }
        }
    }

    func testSteeringWhileItWaitsReachesTheTurn() async throws {
        let daemon = InMemoryDaemon(script: edit)
        let session = try await started(daemon)
        let (store, task) = await watching(daemon, session)
        try await daemon.client.send(session.id, "write the notes", .enqueue)
        await until(store) { $0.isWaiting }

        await store.send(\.binding.draft, "use a heading")
        await store.send(.sendTapped(.steer))
        await until(store) { $0.items.contains(where: { $0.isPerson("use a heading") }) }
        XCTAssertTrue(store.state.isWorking)
        await task.cancel()
    }

    func testUsageAddsUpEachTurn() async {
        let store = store()
        await store.send(
            .received(event("e1", .usage(model: "a", inputTokens: 10, outputTokens: 2, premiumRequests: 1))))
        await store.send(
            .received(event("e2", .usage(model: "b", inputTokens: 5, outputTokens: 3, premiumRequests: 0.5))))
        XCTAssertEqual(
            store.state.usage, Usage(model: "b", inputTokens: 15, outputTokens: 5, premiumRequests: 1.5))
    }

    func testAStoppedTurnSaysSo() async {
        let store = store()
        await store.send(.received(event("e1", .userMessage("go"))))
        await store.send(.received(event("e2", .turnStarted)))
        await store.send(.received(event("e3", .turnEnded(aborted: true))))
        XCTAssertEqual(store.state.items, [.person(id: "e1", text: "go"), .stopped(id: "e3")])
        XCTAssertFalse(store.state.isWorking)
    }
}

extension ChatItem {
    fileprivate func isPerson(_ text: String) -> Bool {
        if case .person(_, text) = self { true } else { false }
    }
}
