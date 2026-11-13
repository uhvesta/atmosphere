import ComposableArchitecture
import HumataDaemonClient
import HumataSession
import ImageIO
import XCTest

@MainActor
final class SessionFeatureTests: XCTestCase {
    private func started(_ daemon: InMemoryDaemon) async throws -> Session {
        try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work", name: "first"))
    }

    private func said(_ events: IdentifiedArrayOf<SessionEvent>) -> [String] {
        events.compactMap { event in
            switch event.kind {
            case .userMessage(let text, _): "user: \(text)"
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
        await store.receive(\.receivedAll, timeout: .seconds(1))
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(said(store.state.events), ["user: earlier", "agent: You said: earlier"])

        await store.send(\.binding.draft, "  hello  ")
        await store.send(.sendTapped(.enqueue)) {
            $0.draft = ""
        }
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(
            said(store.state.events),
            ["user: earlier", "agent: You said: earlier", "user: hello", "agent: You said: hello"])
        XCTAssertFalse(store.state.isWorking)
        await watching.cancel()
    }

    func testTheReviewIsReadAgainWhenATurnEndsWhileItsShown() async throws {
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
        // As the review's view appears.
        await store.send(.review(.appeared))
        await store.receive(\.review.loaded)
        XCTAssertEqual(store.state.review.files.map(\.path), ["one.txt"])
        // Shown again with no turn since, it isn't read again.
        await store.send(.review(.appeared))
        XCTAssertFalse(store.state.review.loading)

        try await daemon.client.send(session.id, "two", .enqueue)
        await store.receive(\.review.loaded, timeout: .seconds(1))
        XCTAssertEqual(store.state.review.files.map(\.path), ["one.txt", "two.txt"])
        XCTAssertEqual(store.state.review.turns, 2)
        await watching.cancel()
    }

    func testTheReviewIsReadAgainWhenATurnEndedWhileAnotherTabWasShown() async throws {
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
        await store.send(.review(.appeared))
        await store.receive(\.review.loaded)
        await store.send(.tabPicked(.chat))

        try await daemon.client.send(session.id, "two", .enqueue)
        // Until the second turn has ended, on the Chat tab.
        for _ in 0..<100
        where store.state.events.filter({ if case .turnEnded = $0.kind { true } else { false } }).count < 2 {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertEqual(store.state.review.files.map(\.path), ["one.txt"])
        await store.send(.tabPicked(.review))
        await store.send(.review(.appeared))
        await store.receive(\.review.loaded, timeout: .seconds(1))
        XCTAssertEqual(store.state.review.files.map(\.path), ["one.txt", "two.txt"])
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

        await store.send(.forkTapped(before: two.id, chatOnly: false)) {
            $0.forking = true
        }
        let fork = try XCTUnwrap(daemon.sessions.last)
        XCTAssertEqual(fork.parentID, session.id)
        XCTAssertEqual(fork.forkedBeforeEventID, two.id)
        await store.receive(\.forked, fork) {
            $0.forking = false
        }
        await store.receive(\.delegate.openSession, fork)
        XCTAssertEqual(daemon.history(fork.id), Array(daemon.history(session.id).prefix { $0.id != two.id }))
    }

    func testRewindingCutsTheChatPutsTheMessageBackAndUndoBringsItAllBack() async throws {
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
        store.exhaustivity = .off

        let watching = await store.send(.task)
        await store.receive(\.receivedAll, timeout: .seconds(1))
        await store.send(.rewindTapped(two.id))
        XCTAssertNotNil(store.state.rewindDialog)
        await store.send(.rewindDialog(.presented(.chat(two.id))))
        await store.receive(\.rewound, timeout: .seconds(1))
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(store.state.draft, "two")
        XCTAssertEqual(said(store.state.events), ["user: one", "agent: You said: one"])
        let cut = try XCTUnwrap(store.state.events.last?.id)
        XCTAssertEqual(store.state.items.last, .rewound(id: cut, files: false, undoable: true))

        await store.send(.undoRewindTapped)
        await store.receive(\.rewindUndone, timeout: .seconds(1))
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(store.state.draft, "")
        XCTAssertEqual(
            said(store.state.events), ["user: one", "agent: You said: one", "user: two", "agent: You said: two"])
        await watching.cancel()
    }

    func testARewindCantBeUndoneOnceAMessageIsSent() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        try await daemon.client.send(session.id, "one", .enqueue)
        let one = try XCTUnwrap(daemon.history(session.id).first { $0.kind == .userMessage("one") })
        try await daemon.client.rewind(session.id, one.id, false)
        try await daemon.client.send(session.id, "again", .enqueue)

        do {
            try await daemon.client.undoRewind(session.id)
            XCTFail("undid a rewind after a message was sent")
        } catch {}
        let items = ChatItem.items(daemon.history(session.id))
        XCTAssertFalse(items.contains { if case .rewound(_, _, true) = $0 { true } else { false } })
    }

    func testOnlyThePersonsMessagesCanBeRewoundTo() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        try await daemon.client.send(session.id, "one", .enqueue)
        let reply = try XCTUnwrap(daemon.history(session.id).first { $0.kind == .assistantMessage("You said: one") })
        do {
            try await daemon.client.rewind(session.id, reply.id, false)
            XCTFail("rewound to a reply")
        } catch {}
        XCTAssertEqual(daemon.history(session.id).count { $0.kind == .userMessage("one") }, 1)
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
            $0.forking = true
        }
        // The edit runs as the fork's first turn, so it has moved on since.
        await store.receive(\.forked) {
            $0.forking = false
            $0.editing = nil
            $0.edited = ""
        }
        await store.receive(\.delegate.openSession)
        let fork = try XCTUnwrap(daemon.sessions.last)
        XCTAssertEqual(fork.forkedBeforeEventID, two.id)
        let said = daemon.history(fork.id).compactMap { event -> String? in
            if case .userMessage(let text, _) = event.kind { text } else { nil }
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

    func testALostConnectionIsShownAndWatchedAgainAfterAWait() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let clock = TestClock()
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.watch = { _ in AsyncThrowingStream { $0.finish(throwing: DaemonError.disconnected) } }
            $0.continuousClock = clock
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.watchEnded)
        XCTAssertTrue(store.state.disconnected)

        await clock.advance(by: .seconds(1))
        await store.receive(\.task)
        await store.receive(\.watchEnded)

        await clock.advance(by: .seconds(2))
        await store.receive(\.task)
        await store.receive(\.watchEnded)
        await store.skipInFlightEffects()
    }

    func testAnOfflineHostsHistoryIsWatchedAgainLessOftenEachTime() async throws {
        // The daemon sends what it has kept and ends the watch at once.
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let kept = [
            SessionEvent(id: "e1", time: Date(timeIntervalSince1970: 0), kind: .userMessage("hi")),
            SessionEvent(id: "e2", time: Date(timeIntervalSince1970: 1), kind: .turnEnded(aborted: false)),
        ]
        let clock = TestClock()
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.watch = { _ in
                AsyncThrowingStream { continuation in
                    for event in kept { continuation.yield(event) }
                    continuation.finish()
                }
            }
            $0.daemon.markSeen = { _, _ in }
            $0.continuousClock = clock
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.watchEnded)
        await clock.advance(by: .seconds(1))
        await store.receive(\.task)
        await store.receive(\.watchEnded)
        XCTAssertEqual(store.state.watchRetries, 2)

        // Not after another second: the history it sent again was nothing new.
        await clock.advance(by: .seconds(1))
        await clock.advance(by: .seconds(1))
        await store.receive(\.task)
        await store.receive(\.watchEnded)
        XCTAssertEqual(store.state.watchRetries, 3)
        XCTAssertEqual(store.state.events.count, 2)
        await store.skipInFlightEffects()
    }

    /// A small PNG, as a file of one would hold.
    private func png() throws -> Data {
        let context = try XCTUnwrap(
            CGContext(
                data: nil, width: 8, height: 8, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 8, height: 8))
        let image = try XCTUnwrap(context.makeImage())
        let data = NSMutableData()
        let destination = try XCTUnwrap(CGImageDestinationCreateWithData(data, "public.png" as CFString, 1, nil))
        CGImageDestinationAddImage(destination, image, nil)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
        return data as Data
    }

    func testAnImagePastedInTheBoxGoesWithTheMessage() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        var state = SessionFeature.State(session: session)
        state.readsImages = true
        let store = TestStore(initialState: state) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        let image = try png()

        await store.send(.imagePasted(image))
        XCTAssertEqual(store.state.images.count, 1)
        XCTAssertFalse(store.state.canSend, "nothing goes until the image is ready")
        await store.receive(\.imagePrepared)
        XCTAssertEqual(store.state.images.first?.image, ImageAttachment(mediaType: "image/png", data: image))
        XCTAssertNotNil(store.state.images.first?.thumbnail)
        XCTAssertTrue(store.state.canSend)

        await store.send(.sendTapped(.enqueue))
        XCTAssertTrue(store.state.images.isEmpty)
        await store.skipReceivedActions(strict: false)
        let history = try await daemon.client.history(session.id)
        let names = history.compactMap { event -> [String]? in
            guard case .userMessage(_, let images) = event.kind else { return nil }
            return images
        }
        let name = try XCTUnwrap(names.first?.first)
        let sent = try await daemon.client.readImage(session.id, name)
        XCTAssertEqual(sent.data, image)

        await store.send(.thumbnailNeeded(name))
        await store.receive(\.thumbnailLoaded)
        XCTAssertFalse(store.state.thumbnails[name]?.isEmpty ?? true)
    }

    func testSomethingThatIsntAnImageIsTakenOutWithAnAlert() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = SessionFeature.State(session: session)
        state.readsImages = true
        let store = TestStore(initialState: state) {
            SessionFeature()
        } withDependencies: {
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off

        await store.send(.imagePasted(Data("not an image".utf8)))
        await store.receive(\.imagePrepared)
        XCTAssertTrue(store.state.images.isEmpty)
        XCTAssertNotNil(store.state.alert)
    }

    func testALocalModelThatDoesntSayItSeesImagesTakesNone() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(
            StartRequest(backend: .local, workingDirectory: "/work", name: "local", model: "qwen3:8b"))
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off

        let watching = await store.send(.task)
        await store.receive(\.readsImagesFound, timeout: .seconds(1))
        XCTAssertFalse(store.state.takesImages)
        await store.send(.imagePasted(try png()))
        XCTAssertTrue(store.state.images.isEmpty)
        XCTAssertNotNil(store.state.alert, "it says why")
        await watching.cancel()
    }

    func testAMessageThatWasntSentGoesBackInTheBox() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.send = { _, _, _ in throw DaemonError.disconnected }
        }
        store.exhaustivity = .off

        await store.send(.binding(.set(\.draft, "ship it")))
        await store.send(.sendTapped(.enqueue))
        XCTAssertEqual(store.state.draft, "")
        await store.receive(\.sendFailed)
        XCTAssertEqual(store.state.draft, "ship it")
        XCTAssertNotNil(store.state.alert)
    }

    private func store() -> TestStoreOf<SessionFeature> {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.markSeen = { _, _ in }
        }
        store.exhaustivity = .off
        return store
    }

    func testAnEndedTurnOnScreenIsMarkedSeen() async {
        let seen = LockIsolated<[Int]>([])
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.markSeen = { _, turns in seen.withValue { $0.append(turns) } }
        }
        store.exhaustivity = .off
        await store.send(.received(event("e1", .turnStarted)))
        await store.send(.received(event("e2", .turnEnded(aborted: false))))
        await store.send(.received(event("e3", .turnStarted)))
        await store.send(.received(event("e4", .turnEnded(aborted: false))))
        await store.finish()
        XCTAssertEqual(seen.value.last, 2)
    }

    func testAReplyWhileHumataIsBehindIsSeenOnlyOnceItComesToTheFront() async {
        let seen = LockIsolated<[Int]>([])
        let marked = expectation(description: "the reply marked seen")
        let active = LockIsolated(false)
        let (activations, activate) = AsyncStream.makeStream(of: Void.self)
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.markSeen = { _, turns in
                seen.withValue { $0.append(turns) }
                marked.fulfill()
            }
            $0.daemon.watch = { _ in AsyncThrowingStream { _ in } }
            $0.appActivity = AppActivity(
                isActive: { active.value }, activations: { activations }, setBadge: { _ in })
        }
        store.exhaustivity = .off
        let task = await store.send(.task)
        await store.send(.received(event("e1", .turnStarted)))
        await store.send(.received(event("e2", .turnEnded(aborted: false))))
        XCTAssertEqual(seen.value, [])

        active.setValue(true)
        activate.yield()
        await store.receive(\.becameActive)
        await fulfillment(of: [marked], timeout: 5)
        XCTAssertEqual(seen.value, [1])
        activate.finish()
        await task.cancel()
    }

    private func event(_ id: String, _ kind: SessionEvent.Kind) -> SessionEvent {
        SessionEvent(id: id, time: Date(timeIntervalSince1970: 0), kind: kind)
    }

    func testAWriteShowsWhatItWouldChangeBeforeItIsAllowed() async {
        let store = store()
        await store.send(.received(event("e1", .turnStarted)))
        await store.send(
            .received(
                event(
                    "e2",
                    .toolStarted(
                        callID: "c1", tool: "Edit",
                        argumentsJSON: #"{"file_path":"a.swift","old_string":"let a = 1","#
                            + #""new_string":"let a = 2\nlet b = 3"}"#
                    ))))
        await store.send(
            .received(
                event(
                    "e3",
                    .toolStarted(
                        callID: "c2", tool: "write_file", argumentsJSON: ##"{"path":"b.md","contents":"# B"}"##))))
        await store.send(
            .received(event("e4", .toolStarted(callID: "c3", tool: "shell", argumentsJSON: #"{"command":"ls"}"#))))
        let changes = store.state.items.compactMap { item -> ToolCall.Change?? in
            guard case .tool(let call) = item else { return nil }
            return .some(call.change)
        }
        XCTAssertEqual(
            changes,
            [
                ToolCall.Change(removed: ["let a = 1"], added: ["let a = 2", "let b = 3"]),
                ToolCall.Change(removed: [], added: ["# B"]),
                nil,
            ])
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

    func testARequestTheAgentsJudgeLetThroughSaysSo() async {
        let store = store()
        await store.send(.received(event("e1", .permissionRequested(requestID: "p1", kind: .shell, summary: "ls"))))
        await store.send(
            .received(
                event("e2", .permissionCompleted(requestID: "p1", approved: true, automatic: true, judged: true))))
        let answers = store.state.items.compactMap { item -> Permission.Answer? in
            switch item {
            case .permission(_, let permission): permission.answer
            case .tool(let call): call.permission?.answer
            default: nil
            }
        }
        XCTAssertEqual(answers, [.allowedByJudge])
        XCTAssertTrue(store.state.pendingPermissions.isEmpty)
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
        store.exhaustivity = .off
        await store.receive(\.approvalModeRefused) {
            $0.session.approvalMode = .askEveryTime
        }
        XCTAssertNotNil(store.state.alert)
    }

    func testASecondForkTapWhileTheFirstIsMadeMakesNoSecondFork() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let forks = LockIsolated(0)
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            // Still being made when the second tap comes.
            $0.daemon.fork = { _, _, _, _ in
                forks.withValue { $0 += 1 }
                return try await Task.never()
            }
        }
        store.exhaustivity = .off

        let first = await store.send(.forkTapped(before: nil, chatOnly: true)) {
            $0.forking = true
        }
        await store.send(.forkTapped(before: nil, chatOnly: false))
        await Task.megaYield()
        XCTAssertEqual(forks.value, 1)
        await first.cancel()
    }

    func testAFailedForkCanBeTriedAgain() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.fork = { _, _, _, _ in throw DaemonError.disconnected }
        }
        store.exhaustivity = .off

        await store.send(.forkTapped(before: nil, chatOnly: false))
        await store.receive(\.failed)
        XCTAssertFalse(store.state.forking)
        XCTAssertNotNil(store.state.alert)
    }

    func testAnEditWhoseForkFailsStaysOpen() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        try await daemon.client.send(session.id, "one", .enqueue)
        let one = try XCTUnwrap(daemon.history(session.id).first { $0.kind == .userMessage("one") })
        var state = SessionFeature.State(session: session)
        state.events = IdentifiedArray(uniqueElements: daemon.history(session.id))
        let store = TestStore(initialState: state) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.fork = { _, _, _, _ in throw DaemonError.disconnected }
        }
        store.exhaustivity = .off

        await store.send(.editTapped(one.id))
        await store.send(.binding(.set(\.edited, "uno")))
        await store.send(.editSent)
        await store.receive(\.failed)
        XCTAssertEqual(store.state.editing, one.id)
        XCTAssertEqual(store.state.edited, "uno")
        XCTAssertFalse(store.state.forking)
    }

    func testAnEditWhoseMessageIsntSentOpensTheForkWithItInTheBox() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        try await daemon.client.send(session.id, "one", .enqueue)
        let one = try XCTUnwrap(daemon.history(session.id).first { $0.kind == .userMessage("one") })
        var state = SessionFeature.State(session: session)
        state.events = IdentifiedArray(uniqueElements: daemon.history(session.id))
        let store = TestStore(initialState: state) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.send = { _, _, _ in throw DaemonError.disconnected }
        }
        store.exhaustivity = .off

        await store.send(.editTapped(one.id))
        await store.send(.binding(.set(\.edited, "uno")))
        await store.send(.editSent)
        await store.receive(\.forkSendFailed)
        let fork = try XCTUnwrap(daemon.sessions.last)
        await store.receive(\.delegate.openSession, fork)
        // The fork's message box has the edit, to send from there.
        XCTAssertEqual(SessionFeature.State(session: fork).draft, "uno")
        XCTAssertNil(store.state.editing)
    }

    func testAPermissionIsAnsweredOnce() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let answers = LockIsolated(0)
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.approve = { _, _, _ in answers.withValue { $0 += 1 } }
        }
        store.exhaustivity = .off

        await store.send(.answerPermission(requestID: "r1", approved: true)) {
            $0.answering = ["r1"]
        }
        await store.send(.answerPermission(requestID: "r1", approved: false))
        await store.finish()
        XCTAssertEqual(answers.value, 1)
    }

    func testAFailedAnswerCanBeGivenAgain() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.approve = { _, _, _ in throw DaemonError.disconnected }
        }
        store.exhaustivity = .off

        await store.send(.answerPermission(requestID: "r1", approved: true))
        await store.receive(\.failed)
        XCTAssertEqual(store.state.answering, [])
        XCTAssertNotNil(store.state.alert)
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

    func testAMessageQueuedWhileItWaitsCanBeTakenOut() async throws {
        let daemon = InMemoryDaemon(script: edit)
        let session = try await started(daemon)
        let (store, task) = await watching(daemon, session)
        try await daemon.client.send(session.id, "write the notes", .enqueue)
        await until(store) { $0.isWaiting }

        await store.send(\.binding.draft, "then the changelog")
        await store.send(.sendTapped(.enqueue))
        await until(store) { !$0.queued.isEmpty }
        XCTAssertEqual(store.state.queued.map(\.text), ["then the changelog"])
        let queued = try XCTUnwrap(store.state.queued.first?.id)

        await store.send(.unqueueTapped(queued))
        await until(store) { $0.queued.isEmpty }
        await store.send(.abortTapped)
        await until(store) { !$0.isWorking }
        XCTAssertFalse(store.state.items.contains { $0.isPerson("then the changelog") })
        await task.cancel()
    }

    func testAQueuedMessageLeavesTheQueueWhenItsTurnStarts() async throws {
        let daemon = InMemoryDaemon(script: edit)
        let session = try await started(daemon)
        let (store, task) = await watching(daemon, session)
        try await daemon.client.send(session.id, "write the notes", .enqueue)
        await until(store) { $0.isWaiting }
        try await daemon.client.send(session.id, "then the changelog", .enqueue)
        await until(store) { !$0.queued.isEmpty }

        await store.send(.abortTapped)
        await until(store) { $0.items.contains { $0.isPerson("then the changelog") } }
        XCTAssertEqual(store.state.queued, [])
        await task.cancel()
    }

    func testADraftIsKeptForItsSessionOnly() async {
        let first = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let second = Session(id: "s2", name: "second", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: first)) {
            SessionFeature()
        }
        store.exhaustivity = .off
        await store.send(\.binding.draft, "half written")

        XCTAssertEqual(SessionFeature.State(session: first).draft, "half written")
        XCTAssertEqual(SessionFeature.State(session: second).draft, "")
    }

    func testASentMessageClearsTheKeptDraft() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.send = { _, _, _ in }
        }
        store.exhaustivity = .off
        await store.send(\.binding.draft, "ship it")
        await store.send(.sendTapped(.enqueue))

        XCTAssertEqual(SessionFeature.State(session: session).draft, "")
    }

    func testAQueuedMessageARestartDroppedIsOfferedBackNotSent() async {
        let store = store()
        await store.send(.received(event("e1", .userMessage("write the notes"))))
        await store.send(.received(event("e2", .messageQueued(queueID: "q1", text: "then the changelog"))))
        await store.send(.received(event("e3", .dequeued(queueID: "q1", dropped: true))))
        XCTAssertEqual(store.state.queued, [])
        XCTAssertEqual(store.state.dropped.map(\.text), ["then the changelog"])

        await store.send(.putBackTapped("q1"))
        XCTAssertEqual(store.state.draft, "then the changelog")
        XCTAssertEqual(store.state.dropped, [])
    }

    func testADroppedMessageIsNoLongerOfferedOnceThePersonSendsAgain() async {
        let store = store()
        await store.send(.received(event("e1", .messageQueued(queueID: "q1", text: "then the changelog"))))
        await store.send(.received(event("e2", .dequeued(queueID: "q1", dropped: true))))
        await store.send(.received(event("e3", .userMessage("something else"))))
        XCTAssertEqual(store.state.dropped, [])
    }

    func testAMessageTakenOutOfTheQueueIsNotOfferedBack() async {
        let store = store()
        await store.send(.received(event("e1", .messageQueued(queueID: "q1", text: "then the changelog"))))
        await store.send(.received(event("e2", .dequeued(queueID: "q1"))))
        XCTAssertEqual(store.state.dropped, [])
    }

    /// A store for session `id`, which also names where its draft is kept.
    private func queueStore(
        _ id: String = "s1", _ dependencies: @escaping (inout DependencyValues) -> Void
    ) -> TestStoreOf<SessionFeature> {
        let session = Session(id: id, name: "first", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.markSeen = { _, _ in }
            dependencies(&$0)
        }
        store.exhaustivity = .off
        return store
    }

    func testAQueuedMessageQueuedAgainShowsItsNewText() async {
        let store = store()
        await store.send(.received(event("e1", .messageQueued(queueID: "q1", text: "then the changelog"))))
        await store.send(
            .received(event("e2", .messageQueued(queueID: "q1", text: "then the changelog\n\nand tag it"))))
        XCTAssertEqual(store.state.queued, [QueuedMessage(id: "q1", text: "then the changelog\n\nand tag it")])
    }

    func testAnEditedQueuedMessageIsSavedWithItsNewText() async {
        let saved = LockIsolated<[String]>([])
        let store = queueStore {
            $0.daemon.editQueued = { _, id, text in saved.withValue { $0.append("\(id): \(text)") } }
        }
        await store.send(.received(event("e1", .messageQueued(queueID: "q1", text: "then the changelog"))))
        await store.send(.editQueuedTapped("q1"))
        XCTAssertEqual(store.state.queuedEdit, "then the changelog")
        await store.send(\.binding.queuedEdit, "then the release notes")
        await store.send(.editQueuedSaved)
        await store.finish()
        XCTAssertEqual(saved.value, ["q1: then the release notes"])
        XCTAssertNil(store.state.editingQueued)
    }

    func testAnEditWhoseTurnAlreadyStartedGoesInTheBox() async {
        let store = queueStore("lost-race") {
            $0.daemon.editQueued = { _, _, _ in
                throw DaemonError.refused(.unknownQueuedMessage, message: "gone")
            }
        }
        await store.send(.received(event("e1", .messageQueued(queueID: "q1", text: "then the changelog"))))
        await store.send(.editQueuedTapped("q1"))
        await store.send(\.binding.queuedEdit, "then the release notes")
        await store.send(.editQueuedSaved)
        await store.receive(\.editQueuedFailed)
        XCTAssertEqual(store.state.draft, "then the release notes")
        XCTAssertNotNil(store.state.alert)
    }

    func testAQueuedMessageEditedToNothingIsTakenOut() async {
        let removed = LockIsolated<[String]>([])
        let store = queueStore { $0.daemon.unqueue = { _, id in removed.withValue { $0.append(id) } } }
        await store.send(.received(event("e1", .messageQueued(queueID: "q1", text: "then the changelog"))))
        await store.send(.editQueuedTapped("q1"))
        await store.send(\.binding.queuedEdit, "  ")
        await store.send(.editQueuedSaved)
        await store.finish()
        XCTAssertEqual(removed.value, ["q1"])
    }

    func testStopAndEditStopsTheTurnAndPutsTheMessageBack() async {
        let stopped = LockIsolated(0)
        let store = queueStore("stop-and-edit") { $0.daemon.abort = { _ in stopped.withValue { $0 += 1 } } }
        await store.send(.received(event("e1", .userMessage("write the notes"))))
        await store.send(.received(event("e2", .turnStarted)))
        await store.send(.stopAndEditTapped)
        await store.finish()
        XCTAssertEqual(stopped.value, 1)
        XCTAssertEqual(store.state.draft, "write the notes")
    }

    func testStopAndEditDoesNothingBetweenTurns() async {
        let stopped = LockIsolated(0)
        let store = queueStore("between-turns") { $0.daemon.abort = { _ in stopped.withValue { $0 += 1 } } }
        await store.send(.received(event("e1", .userMessage("write the notes"))))
        await store.send(.received(event("e2", .turnStarted)))
        await store.send(.received(event("e3", .turnEnded(aborted: false))))
        await store.send(.stopAndEditTapped)
        await store.finish()
        XCTAssertEqual(stopped.value, 0)
        XCTAssertEqual(store.state.draft, "")
    }

    func testALongPasteBecomesAChipAndIsSentAfterTheTypedText() async {
        let sent = LockIsolated<[String]>([])
        let store = queueStore("long-paste") {
            $0.daemon.send = { _, text, _ in sent.withValue { $0.append(text) } }
            $0.uuid = .incrementing
        }
        let log = (1...30).map { "line \($0)" }.joined(separator: "\n")
        await store.send(\.binding.draft, "why does this fail? ")
        await store.send(\.binding.draft, "why does this fail? " + log)
        XCTAssertEqual(store.state.draft, "why does this fail? ")
        XCTAssertEqual(store.state.pastes.map(\.text), [log])
        XCTAssertEqual(store.state.pastes.first?.lineCount, 30)

        await store.send(.sendTapped(.enqueue))
        await store.finish()
        XCTAssertEqual(sent.value, ["why does this fail?\n\n" + log])
        XCTAssertEqual(store.state.pastes, [])
    }

    func testTypingAndShortPastesStayInTheBox() async {
        let store = queueStore("short-paste") { $0.uuid = .incrementing }
        await store.send(\.binding.draft, "fix the build")
        await store.send(\.binding.draft, "fix the build\nand the tests")
        XCTAssertEqual(store.state.draft, "fix the build\nand the tests")
        XCTAssertEqual(store.state.pastes, [])
    }

    func testAQuoteGoesAfterWhatsTypedAndLeavesRoomToWrite() async {
        let store = queueStore("quote") { $0.uuid = .incrementing }
        await store.send(\.binding.draft, "look at this")
        await store.send(.quoted("```\nlet x = 1\n```"))
        XCTAssertEqual(store.state.draft, "look at this\n\n```\nlet x = 1\n```\n\n")
        XCTAssertTrue(store.state.canSend)
    }

    func testARemovedPasteIsNotSent() async {
        let store = queueStore("removed-paste") { $0.uuid = .incrementing }
        await store.send(\.binding.draft, String(repeating: "x", count: 2_000))
        let id = store.state.pastes.first?.id ?? ""
        await store.send(.removePasteTapped(id))
        XCTAssertEqual(store.state.pastes, [])
        XCTAssertFalse(store.state.canSend)
    }

    func testAPasteInTheMiddleLeavesTheTextAroundIt() {
        let pasted = String(repeating: "y", count: 1_500)
        let taken = Paste.takeOut(old: "before after", new: "before " + pasted + "after")
        XCTAssertEqual(taken?.draft, "before after")
        XCTAssertEqual(taken?.pasted, pasted)
    }

    func testAReconnectsHistoryKeepsTheReplyStreamingInWithoutDoublingIt() async {
        let store = store()
        let history = [
            event("e1", .userMessage("go")), event("e2", .turnStarted),
            event("d1", .assistantDelta("Hel")), event("d2", .assistantDelta("lo")),
        ]
        await store.send(.receivedAll(history))
        XCTAssertEqual(store.state.streaming, "Hello")
        // The watch drops; watching again sends the history once more.
        await store.send(.receivedAll(history + [event("d3", .assistantDelta(" there"))]))
        XCTAssertEqual(store.state.streaming, "Hello there")
        await store.send(.received(event("e3", .assistantMessage("Hello there"))))
        XCTAssertEqual(store.state.streaming, "")
    }

    func testLiveEventsShowTheSameChatAsTheirHistory() async {
        let events = [
            event("e1", .userMessage("go")), event("e2", .turnStarted),
            event("e3", .toolStarted(callID: "c1", tool: "Bash", argumentsJSON: "{}")),
            event("e4", .permissionRequested(requestID: "r1", kind: .shell, summary: "ls")),
            event("e5", .permissionCompleted(requestID: "r1", approved: true, automatic: false, judged: false)),
            event("e6", .toolCompleted(callID: "c1", success: true, output: "a")),
            event("e7", .assistantMessage("done")), event("e8", .turnEnded(aborted: false)),
        ]
        let live = store()
        for one in events { await live.send(.received(one)) }
        let loaded = store()
        await loaded.send(.receivedAll(events))
        XCTAssertEqual(live.state.items, loaded.state.items)
        XCTAssertEqual(live.state.items, ChatItem.items(events))
    }

    func testTheChatLoadsUntilItsHistoryComes() async {
        let store = store()
        XCTAssertFalse(store.state.historyLoaded)
        await store.send(.receivedAll([]))
        XCTAssertTrue(store.state.historyLoaded)
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

    func testEachTurnShowsWhatItUsedWhereItEnds() {
        let events = [
            event("e1", .userMessage("go")), event("e2", .turnStarted),
            event("e3", .usage(model: "a", inputTokens: 10, outputTokens: 2, premiumRequests: 1)),
            event("e4", .usage(model: "a", inputTokens: 5, outputTokens: 3, premiumRequests: 0)),
            event("e5", .assistantMessage("done")), event("e6", .turnEnded(aborted: false)), event("e7", .idle),
            event("e8", .userMessage("again")), event("e9", .turnStarted), event("e10", .turnEnded(aborted: true)),
        ]
        let costs = ChatItem.items(events).compactMap { item -> Usage? in
            if case .cost(_, let usage) = item { usage } else { nil }
        }
        XCTAssertEqual(costs, [Usage(model: "a", inputTokens: 15, outputTokens: 5, premiumRequests: 1)])
        guard case .cost? = ChatItem.items(events.prefix(7)).last else { return XCTFail("no cost at the end") }
    }

    func testAStoppedTurnSaysSo() async {
        let store = store()
        await store.send(.received(event("e1", .userMessage("go"))))
        await store.send(.received(event("e2", .turnStarted)))
        await store.send(.received(event("e3", .turnEnded(aborted: true))))
        XCTAssertEqual(store.state.items, [.person(id: "e1", text: "go"), .stopped(id: "e3")])
        XCTAssertFalse(store.state.isWorking)
    }

    func testARequestLeftWhenItsTurnEndsIsNoLongerWaiting() async {
        let store = store()
        await store.send(
            .receivedAll([
                event("e1", .userMessage("go")),
                event("e2", .turnStarted),
                event("e3", .permissionRequested(requestID: "p1", kind: .shell, summary: "touch SPEC.md")),
                event("e4", .turnEnded(aborted: true)),
            ]))
        XCTAssertFalse(store.state.isWaiting)
        let answers = store.state.items.compactMap { item in
            if case .permission(_, let permission) = item { permission.answer } else { nil }
        }
        XCTAssertEqual(answers, [.notAnswered])

        await store.send(.received(event("e5", .userMessage("hello"))))
        await store.send(.received(event("e6", .turnStarted)))
        XCTAssertFalse(store.state.isWaiting)
    }

    func testAHistorySentAgainInTwoBatchesKeepsTheReplyBeingWritten() async {
        let store = store()
        let before = [
            event("e1", .userMessage("hi")),
            event("e2", .turnStarted),
            event("e3", .assistantMessage("Hello.")),
            event("e4", .turnEnded(aborted: false)),
            event("e5", .userMessage("more")),
            event("e6", .turnStarted),
        ]
        await store.send(.receivedAll(before + [event("d1", .assistantDelta("Writ"))]))
        XCTAssertEqual(store.state.streaming, "Writ")
        // After a reconnect: the older replies come first, then the part.
        await store.send(.receivedAll(Array(before.prefix(4))))
        await store.send(.receivedAll(Array(before.suffix(2)) + [event("d1", .assistantDelta("Writ"))]))
        XCTAssertEqual(store.state.streaming, "Writ")
        await store.send(.received(event("d2", .assistantDelta("ing"))))
        XCTAssertEqual(store.state.streaming, "Writing")
    }

    func testEventsTakenTogetherShowTheSameChatAsOneByOne() async {
        let events = [
            event("e1", .userMessage("go")),
            event("e2", .turnStarted),
            event("d1", .assistantDelta("Do")),
            event("d2", .assistantDelta("ne")),
            event("e3", .assistantMessage("Done")),
            event("e4", .turnEnded(aborted: false)),
        ]
        let together = store()
        await together.send(.receivedAll(events))
        let oneByOne = store()
        for event in events { await oneByOne.send(.received(event)) }
        XCTAssertEqual(together.state.items, oneByOne.state.items)
        XCTAssertEqual(together.state.streaming, "")
        XCTAssertFalse(together.state.isWorking)
    }

    func testForkingFromAHighlightedReplyKeepsItsWholeTurn() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        try await daemon.client.send(session.id, "one", .enqueue)
        try await daemon.client.send(session.id, "two", .enqueue)
        let history = daemon.history(session.id)
        let reply = try XCTUnwrap(history.first { $0.kind == .assistantMessage("You said: one") })
        var state = SessionFeature.State(session: session, highlighted: reply.id)
        state.events = IdentifiedArray(uniqueElements: history)
        XCTAssertEqual(state.highlightedItem, reply.id)
        let store = TestStore(initialState: state) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }

        await store.send(.forkFromHighlightTapped)
        await store.receive(\.forkTapped) {
            $0.forking = true
        }
        await store.receive(\.forked) {
            $0.forking = false
        }
        await store.receive(\.delegate.openSession)
        let fork = try XCTUnwrap(daemon.sessions.last)
        // The reply's turn whole, and nothing of the next.
        let kept = daemon.history(fork.id)
        XCTAssertTrue(kept.contains { $0.kind == .turnEnded(aborted: false) })
        XCTAssertEqual(said(IdentifiedArray(uniqueElements: kept)), ["user: one", "agent: You said: one"])

        await store.send(.highlightDismissed) {
            $0.highlighted = nil
        }
    }

    func testAHighlightedToolOutputIsShownOnItsCallsRowAndTheLastTurnForksWhole() {
        let session = Session(id: "s", name: "s", backend: .scripted, workingDirectory: "/work")
        let now = Date(timeIntervalSince1970: 0)
        let events = [
            SessionEvent(id: "e1", time: now, kind: .userMessage("test it")),
            SessionEvent(id: "e2", time: now, kind: .toolStarted(callID: "c1", tool: "shell", argumentsJSON: "{}")),
            SessionEvent(id: "e3", time: now, kind: .toolCompleted(callID: "c1", success: true, output: "ok")),
            SessionEvent(id: "e4", time: now, kind: .turnEnded(aborted: false)),
        ]
        var state = SessionFeature.State(session: session, highlighted: "e3")
        XCTAssertNil(state.highlightedItem)
        state.events = IdentifiedArray(uniqueElements: events)
        XCTAssertEqual(state.highlightedItem, "c1")
        XCTAssertNil(state.forkPointAfterHighlight)
    }

    private func store(_ state: SessionFeature.State) -> TestStoreOf<SessionFeature> {
        let store = TestStore(initialState: state) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.markSeen = { _, _ in }
        }
        store.exhaustivity = .off
        return store
    }

    func testApprovingAMoveFromTheSessionAnswersItsRequest() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = SessionFeature.State(session: session)
        state.request = AgentRequest(
            id: "r1", sessionID: "s1", kind: .move(item: "i1", status: .inReview), text: "Done",
            created: Date(timeIntervalSince1970: 0))
        let store = store(state)
        await store.send(.moveApproved)
        await store.receive(\.delegate.answerRequest)
    }

    func testShowingProposalsScrollsToTheFirstOneWaiting() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = SessionFeature.State(session: session)
        state.proposals.proposals = [
            Proposal(id: "p1", title: "Done", state: .declined),
            Proposal(id: "p2", title: "Log in"),
            Proposal(id: "p3", title: "Log out"),
        ]
        let store = store(state)
        await store.send(.showProposalsTapped) {
            $0.scrollTarget = "p2"
        }
        await store.send(.scrolled) {
            $0.scrollTarget = nil
        }
    }

    func testTryingAgainAfterAFailedTurnSendsTheLastMessageAgain() async {
        let sent = LockIsolated<[String]>([])
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.send = { _, text, _ in sent.withValue { $0.append(text) } }
            $0.daemon.markSeen = { _, _ in }
        }
        store.exhaustivity = .off
        await store.send(.received(event("e1", .userMessage("first try"))))
        await store.send(.received(event("e2", .userMessage("Fix the login"))))
        await store.send(.received(event("e3", .turnStarted)))
        await store.send(.received(event("e4", .failed("The model went away"))))
        await store.send(.received(event("e5", .turnEnded(aborted: false))))
        XCTAssertTrue(store.state.lastTurnFailed)

        await store.send(.retryTapped)
        await store.finish()
        XCTAssertEqual(sent.value, ["Fix the login"])
    }

    func testReplacingOneLineOfASpecTakesOneOutAndPutsOneIn() {
        let change = ToolCall.Change.between("# Log in\nWith email.\nFast.", "# Log in\nWith a passkey.\nFast.")
        XCTAssertEqual(change, ToolCall.Change(removed: ["With email."], added: ["With a passkey."]))
    }
}

extension ChatItem {
    fileprivate func isPerson(_ text: String) -> Bool {
        if case .person(_, text, _) = self { true } else { false }
    }
}
