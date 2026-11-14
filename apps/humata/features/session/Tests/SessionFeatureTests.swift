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

    /// The message the session handed on to be sent, once it has.
    private func handedOn(_ store: TestStoreOf<SessionFeature>) async -> SessionFeature.Outgoing? {
        var message: SessionFeature.Outgoing?
        await store.receive({ action in
            guard case .delegate(.send(let outgoing)) = action else { return false }
            message = outgoing
            return true
        })
        return message
    }

    /// Sends what the session handed on, as the window showing it does.
    private func deliver(_ store: TestStoreOf<SessionFeature>, to daemon: InMemoryDaemon) async throws {
        let handed = await handedOn(store)
        let message = try XCTUnwrap(handed)
        let images = message.images.compactMap(\.image)
        if images.isEmpty {
            try await daemon.client.send(message.session, message.text, message.mode)
        } else {
            try await daemon.client.sendImages(message.session, message.text, images, message.mode)
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
            $0.continuousClock = ImmediateClock()
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
        try await deliver(store, to: daemon)
        await until(store) { self.said($0.events).count == 4 }
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
            $0.continuousClock = ImmediateClock()
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
            $0.continuousClock = ImmediateClock()
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

    func testAToolCallOpensAndClosesAndOthersStayAsTheyAre() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        let store = TestStore(initialState: SessionFeature.State(session: session)) { SessionFeature() }

        await store.send(.toolToggled(id: "read")) { $0.expandedTools = ["read"] }
        await store.send(.toolToggled(id: "grep")) { $0.expandedTools = ["read", "grep"] }
        await store.send(.toolToggled(id: "read")) { $0.expandedTools = ["grep"] }
    }

    func testABlankMessageIsNotSent() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = SessionFeature.State(session: session)
        state.draft = "   "
        // Were it handed on to be sent, the store would have that to receive.
        let store = TestStore(initialState: state) {
            SessionFeature()
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
            $0.continuousClock = ImmediateClock()
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
            $0.editCaret = true
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
            $0.editCaret = false
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

    func testAHistoryReadFromDiskShowsHowMuchIsReadUntilItComes() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let said = SessionEvent(id: "e1", time: Date(timeIntervalSince1970: 0), kind: .userMessage("hi"))
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.chatCommands = { _ in ChatCommands() }
            $0.daemon.watch = { _, _ in
                AsyncThrowingStream { continuation in
                    continuation.yield(.loading(read: 0, total: 0))
                    continuation.yield(.loading(read: 40, total: 100))
                    continuation.yield(.event(said))
                    continuation.yield(.historyLoaded)
                }
            }
            $0.daemon.markSeen = { _, _ in }
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.historyLoading)
        XCTAssertNil(store.state.historyLoading?.fraction)
        await store.receive(\.historyLoading)
        XCTAssertEqual(store.state.historyLoading?.fraction, 0.4)
        XCTAssertFalse(store.state.historyLoaded)
        await store.receive(\.receivedAll)
        XCTAssertNil(store.state.historyLoading)
        XCTAssertTrue(store.state.historyLoaded)
        await store.skipInFlightEffects()
    }

    func testALostConnectionIsShownAndWatchedAgainAfterAWait() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let clock = TestClock()
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.chatCommands = { _ in ChatCommands() }
            $0.daemon.watch = { _, _ in AsyncThrowingStream { $0.finish(throwing: DaemonError.disconnected) } }
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

    func testAChatWatchedAgainIsSentOnlyWhatCameAfterItsLastEvent() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let event = { (id: String, kind: SessionEvent.Kind) in
            SessionEvent(id: id, time: Date(timeIntervalSince1970: 0), kind: kind)
        }
        let history = LockIsolated([event("e1", .userMessage("hi")), event("e2", .turnEnded(aborted: false))])
        let asked = LockIsolated<[String?]>([])
        let clock = TestClock()
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            // As the daemon does: the history past `after`, then the
            // connection drops.
            $0.daemon.chatCommands = { _ in ChatCommands() }
            $0.daemon.watch = { _, after in
                asked.withValue { $0.append(after) }
                let events = history.value
                let unsent = after.flatMap { after in events.lastIndex { $0.id == after } }.map { $0 + 1 } ?? 0
                return AsyncThrowingStream { continuation in
                    for event in events[unsent...] { continuation.yield(.event(event)) }
                    continuation.yield(.historyLoaded)
                    continuation.finish(throwing: DaemonError.disconnected)
                }
            }
            $0.daemon.markSeen = { _, _ in }
            $0.continuousClock = clock
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.watchEnded)
        history.withValue { $0.append(event("e3", .userMessage("more"))) }
        await clock.advance(by: .seconds(1))
        await store.receive(\.task)
        await store.receive(\.watchEnded)

        XCTAssertEqual(asked.value, [nil, "e2"])
        XCTAssertEqual(store.state.events.map(\.id), ["e1", "e2", "e3"])
        await store.skipInFlightEffects()
    }

    func testAReplyBeingWrittenIsTakenInAtMostOnceAFrame() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let event = { (id: String, kind: SessionEvent.Kind) in
            SessionEvent(id: id, time: Date(timeIntervalSince1970: 0), kind: kind)
        }
        let (stream, live) = AsyncThrowingStream<WatchItem, Error>.makeStream()
        let clock = TestClock()
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.chatCommands = { _ in ChatCommands() }
            $0.daemon.watch = { _, _ in stream }
            $0.daemon.markSeen = { _, _ in }
            $0.continuousClock = clock
        }
        store.exhaustivity = .off

        await store.send(.task)
        live.yield(.historyLoaded)
        await store.receive(\.historyReplayed)
        live.yield(.event(event("d1", .assistantDelta("Hel"))))
        await store.receive(\.receivedAll)
        XCTAssertEqual(store.state.streaming, "Hel")

        // The words that come within the frame wait for its end, and go
        // together.
        live.yield(.event(event("d2", .assistantDelta("lo"))))
        live.yield(.event(event("d3", .assistantDelta(" there"))))
        await Task.megaYield()
        await clock.advance(by: SessionFeature.streamingFrame / 2)
        XCTAssertEqual(store.state.streaming, "Hel")
        await clock.advance(by: SessionFeature.streamingFrame)
        await store.receive(\.receivedAll)
        XCTAssertEqual(store.state.streaming, "Hello there")

        live.finish()
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
            $0.daemon.chatCommands = { _ in ChatCommands() }
            $0.daemon.watch = { _, _ in
                AsyncThrowingStream { continuation in
                    for event in kept { continuation.yield(.event(event)) }
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
        try await deliver(store, to: daemon)
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

    func testSendingEmptiesTheBoxAndAMessageThatWasntSentGoesBackInIt() async throws {
        let session = Session(id: "hand-on", name: "first", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        }
        store.exhaustivity = .off

        await store.send(.binding(.set(\.draft, "ship it")))
        await store.send(.sendTapped(.enqueue))
        XCTAssertEqual(store.state.draft, "")
        let handed = await handedOn(store)
        let message = try XCTUnwrap(handed)
        XCTAssertEqual(message.session, session.id)
        XCTAssertEqual(message.text, "ship it")
        XCTAssertEqual(message.mode, .enqueue)

        await store.send(
            .sendFailed(
                text: message.typed, pastes: message.pastes, images: message.images,
                message: "The daemon isn’t running."))
        XCTAssertEqual(store.state.draft, "ship it")
        XCTAssertNotNil(store.state.alert)
    }

    func testAMessageShowsAsBeingSentUntilItIsAnsweredFor() async {
        let store = queueStore("sending") { _ in }
        await store.send(\.binding.draft, "ship it")
        await store.send(.sendTapped(.enqueue))
        XCTAssertEqual(store.state.sending, 1)
        await store.send(\.binding.draft, "and this")
        await store.send(.sendTapped(.enqueue))
        XCTAssertEqual(store.state.sending, 2)

        await store.send(.sent)
        XCTAssertEqual(store.state.sending, 1)
        await store.send(.sendFailed(text: "and this", pastes: [], images: [], message: "The daemon isn’t running."))
        XCTAssertEqual(store.state.sending, 0)
        // An answer for a message sent before the session was opened again.
        await store.send(.sent)
        XCTAssertEqual(store.state.sending, 0)
    }

    func testAMessageThatWasntSentGoesBackAheadOfWhatWasPutInTheBoxSince() async {
        let session = Session(id: "send-failed", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = SessionFeature.State(session: session)
        state.pastes = [Paste(id: "p2", text: "pasted since")]
        state.images = [DraftImage(id: "i2", name: "since.png")]
        let store = store(state)

        await store.send(.binding(.set(\.draft, "and one more thing")))
        await store.send(
            .sendFailed(
                text: "ship it", pastes: [Paste(id: "p1", text: "pasted before")],
                images: [DraftImage(id: "i1", name: "before.png"), DraftImage(id: "i2", name: "since.png")],
                message: "The daemon isn’t running."))
        XCTAssertEqual(store.state.draft, "ship it\n\nand one more thing")
        XCTAssertEqual(store.state.pastes.map(\.id), ["p1", "p2"])
        XCTAssertEqual(store.state.images.map(\.id), ["i1", "i2"])
        XCTAssertNotNil(store.state.alert)

        // The draft is kept as it now is, for the session opened again.
        XCTAssertEqual(SessionFeature.State(session: session).draft, "ship it\n\nand one more thing")
    }

    /// A store whose watch stays open and sends nothing, so a test says
    /// what comes.
    private func watching(_ state: SessionFeature.State? = nil) -> TestStoreOf<SessionFeature> {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let store = TestStore(initialState: state ?? SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.chatCommands = { _ in ChatCommands() }
            $0.daemon.watch = { _, _ in AsyncThrowingStream { _ in } }
            $0.daemon.markSeen = { _, _ in }
        }
        store.exhaustivity = .off
        return store
    }

    func testARewindInTheHistoryOfAChatJustOpenedIsntReadAgain() async {
        let store = watching()
        await store.send(.task)
        await store.send(.receivedAll([event("e1", .userMessage("one"))]))
        await store.send(.receivedAll([event("r1", .rewound(text: "two", files: false))]))
        XCTAssertEqual(store.state.events.map(\.id), ["e1", "r1"])
        await store.skipInFlightEffects()
    }

    func testARewindAfterTheHistoryCameIsReadAgainOnce() async {
        let history = [event("e1", .userMessage("one"))]
        let rewound = event("r1", .rewound(text: "two", files: false))
        let store = watching()
        await store.send(.task)
        await store.send(.receivedAll(history))
        await store.send(.historyReplayed)
        await store.send(.receivedAll([rewound]))
        await store.receive(\.task)
        XCTAssertTrue(store.state.events.isEmpty)

        // The history read again holds the rewind, and stays.
        await store.send(.receivedAll(history + [rewound]))
        await store.send(.historyReplayed)
        XCTAssertEqual(store.state.events.map(\.id), ["e1", "r1"])
        await store.skipInFlightEffects()
    }

    func testARewindInAHistorySentAgainToAChatAlreadyShownIsReadAgain() async {
        let history = [event("e1", .userMessage("one")), event("e2", .userMessage("two"))]
        let store = watching()
        await store.send(.receivedAll(history))
        await store.send(.task)
        await store.send(.receivedAll([history[0], event("r1", .rewound(text: "two", files: false))]))
        await store.receive(\.task)
        XCTAssertTrue(store.state.events.isEmpty)
        await store.skipInFlightEffects()
    }

    func testAWatchSaysWhenItsHistoryHasComeSoALaterRewindIsReadAgain() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let (stream, continuation) = AsyncThrowingStream<WatchItem, any Error>.makeStream()
        let watches = LockIsolated(0)
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.chatCommands = { _ in ChatCommands() }
            $0.daemon.watch = { _, _ in
                watches.withValue { $0 += 1 }
                return watches.value == 1 ? stream : AsyncThrowingStream { _ in }
            }
            $0.daemon.markSeen = { _, _ in }
        }
        store.exhaustivity = .off
        continuation.yield(.event(event("e1", .userMessage("one"))))
        continuation.yield(.event(event("r1", .rewound(text: "two", files: false))))
        continuation.yield(.event(event("u1", .rewindUndone)))
        continuation.yield(.historyLoaded)

        await store.send(.task)
        await store.receive(\.historyReplayed)
        XCTAssertEqual(store.state.events.map(\.id), ["e1", "r1", "u1"])

        continuation.yield(.event(event("r2", .rewound(text: "one", files: false))))
        await store.receive(\.task)
        XCTAssertTrue(store.state.events.isEmpty)
        await store.skipInFlightEffects()
    }

    func testWhatWasBeingWrittenIsKeptWhenASessionIsLeft() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = SessionFeature.State(session: session)
        state.tab = .files
        state.specs.pages = [SpecsFeature.Page(id: "i1", kind: .task, title: "Log in", text: "With email.")]
        state.docs.selected = "notes.md"
        state.docs.comment = "Say which one."
        state.editingQueued = "q1"
        state.queuedEdit = "and add tests"
        let store = store(state)
        await store.send(.specs(.editTapped("i1")))
        await store.send(.specs(.draftChanged("With a passkey.")))
        await store.send(.docs(.lineTapped(2, extending: false)))

        var opened = SessionFeature.State(session: session)
        opened.left = store.state.left
        XCTAssertEqual(opened.left, store.state.left)
        XCTAssertEqual(opened.tab, .files)
        XCTAssertEqual(opened.specs.draft?.pageID, "i1")
        XCTAssertEqual(opened.specs.draft?.text, "With a passkey.")
        XCTAssertEqual(opened.docs.selected, "notes.md")
        XCTAssertEqual(opened.docs.picked?.startLine, 2)
        XCTAssertEqual(opened.docs.comment, "Say which one.")
        XCTAssertEqual(opened.editingQueued, "q1")
        XCTAssertEqual(opened.queuedEdit, "and add tests")
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
            $0.daemon.chatCommands = { _ in ChatCommands() }
            $0.daemon.watch = { _, _ in AsyncThrowingStream { _ in } }
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
            $0.continuousClock = ImmediateClock()
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
        try await deliver(store, to: daemon)
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

    func testAnEditWhoseMessageIsntSentSaysSoThenOpensTheForkWithItInTheBox() async throws {
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
        // Said first, in the session the edit was made in: no fork opens
        // under the alert.
        XCTAssertNotNil(store.state.alert)
        XCTAssertNil(store.state.editing)
        XCTAssertFalse(store.state.forking)
        await store.finish()

        await store.send(.alert(.dismiss))
        await store.receive(\.delegate.openSession, fork)
        // The fork's message box has the edit, to send from there.
        XCTAssertEqual(SessionFeature.State(session: fork).draft, "uno")
        XCTAssertNil(store.state.alert)
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
        try await deliver(store, to: daemon)
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
        try await deliver(store, to: daemon)
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
        let caret = store.state.boxFocus
        await store.send(.editQueuedSaved)
        await store.finish()
        XCTAssertEqual(saved.value, ["q1: then the release notes"])
        XCTAssertNil(store.state.editingQueued)
        XCTAssertNotEqual(store.state.boxFocus, caret, "the box takes the caret the editor had")
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

    func testAnEditOfAQueuedMessageWhoseTurnStartsGoesInTheBox() async {
        let store = queueStore("edit-left-queue") { _ in }
        await store.send(
            .receivedAll(
                [event("e1", .messageQueued(queueID: "q1", text: "then the changelog"))], folded: nil))
        await store.send(.editQueuedTapped("q1"))
        await store.send(\.binding.queuedEdit, "then the release notes")
        await store.send(.received(event("e2", .dequeued(queueID: "q1"))))
        XCTAssertNil(store.state.editingQueued)
        XCTAssertEqual(store.state.queuedEdit, "")
        XCTAssertEqual(store.state.draft, "then the release notes")
        XCTAssertNotNil(store.state.alert)
    }

    func testAnUnchangedEditOfAQueuedMessageWhoseTurnStartsJustCloses() async {
        let store = queueStore("edit-left-queue-unchanged") { _ in }
        await store.send(
            .receivedAll(
                [event("e1", .messageQueued(queueID: "q1", text: "then the changelog"))], folded: nil))
        await store.send(.editQueuedTapped("q1"))
        await store.send(.received(event("e2", .dequeued(queueID: "q1"))))
        XCTAssertNil(store.state.editingQueued)
        XCTAssertEqual(store.state.draft, "")
        XCTAssertNil(store.state.alert)
    }

    func testAKeptEditOfAQueuedMessageThatRanMeanwhileGoesInTheBox() async {
        let store = queueStore("edit-left-queue-away") { _ in }
        // As kept when the session was looked away from.
        await store.send(\.binding.editingQueued, "q1")
        await store.send(\.binding.queuedEdit, "then the release notes")
        await store.send(
            .receivedAll(
                [
                    event("e1", .messageQueued(queueID: "q1", text: "then the changelog")),
                    event("e2", .dequeued(queueID: "q1")),
                ], folded: nil))
        await store.send(.historyReplayed)
        XCTAssertNil(store.state.editingQueued)
        XCTAssertEqual(store.state.draft, "then the release notes")
    }

    func testAQueuedMessageOfOnlyImagesEditedToNothingStaysQueued() async {
        let removed = LockIsolated<[String]>([])
        let store = queueStore("edit-images-only") {
            $0.daemon.unqueue = { _, id in removed.withValue { $0.append(id) } }
        }
        await store.send(.received(event("e1", .messageQueued(queueID: "q1", text: "", images: ["a.png"]))))
        await store.send(.editQueuedTapped("q1"))
        await store.send(.editQueuedSaved)
        await store.finish()
        XCTAssertEqual(removed.value, [])
        XCTAssertEqual(store.state.queued.map(\.id), ["q1"])
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
        let store = queueStore("long-paste") { $0.uuid = .incrementing }
        let log = (1...30).map { "line \($0)" }.joined(separator: "\n")
        await store.send(\.binding.draft, "why does this fail? ")
        await store.send(\.binding.draft, "why does this fail? " + log)
        XCTAssertEqual(store.state.draft, "why does this fail? ")
        XCTAssertEqual(store.state.pastes.map(\.text), [log])
        XCTAssertEqual(store.state.pastes.first?.lineCount, 30)

        await store.send(.sendTapped(.enqueue))
        let message = await handedOn(store)
        XCTAssertEqual(message?.text, "why does this fail?\n\n" + log)
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

    func testAPasteOverALongerSelectionIsStillTakenOut() {
        let pasted = String(repeating: "line\n", count: 30)
        let selected = String(repeating: "z", count: 4_000)
        let taken = Paste.takeOut(old: "see " + selected + " here", new: "see " + pasted + " here")
        XCTAssertEqual(taken?.draft, "see  here")
        XCTAssertEqual(taken?.pasted, pasted)
    }

    func testAPasteBesideAccentsAndEmojiTakesWholeCharacters() {
        // The two flags share their first half.
        let pasted = "🇫🇷" + String(repeating: "y", count: 1_500) + "🇫🇷"
        let taken = Paste.takeOut(old: "é🇫🇮", new: "é" + pasted + "🇫🇮")
        XCTAssertEqual(taken?.draft, "é🇫🇮")
        XCTAssertEqual(taken?.pasted, pasted)
        // "©" and "é" share their last byte.
        let over = String(repeating: "y", count: 1_500) + "é"
        let replaced = Paste.takeOut(old: "a©", new: "a" + over)
        XCTAssertEqual(replaced?.draft, "a")
        XCTAssertEqual(replaced?.pasted, over)
    }

    func testTypingInALongDraftIsNotAPaste() {
        let draft = String(repeating: "word é ", count: 2_000)
        XCTAssertNil(Paste.takeOut(old: draft, new: draft + "s"))
        XCTAssertNil(Paste.takeOut(old: draft + "s", new: draft))
        XCTAssertNil(Paste.takeOut(old: draft, new: "a short line\n" + draft))
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

    func testANewSessionWithNoHistoryStopsLoading() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.continuousClock = ImmediateClock()
        }
        store.exhaustivity = .off

        let watching = await store.send(.task)
        XCTAssertFalse(store.state.historyLoaded)
        await store.receive(\.receivedAll, timeout: .seconds(1))
        XCTAssertTrue(store.state.historyLoaded)
        XCTAssertTrue(store.state.items.isEmpty)
        await watching.cancel()
    }

    func testTheHistorysEndSaidAgainChangesNothing() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let said = SessionEvent(id: "e1", time: Date(timeIntervalSince1970: 0), kind: .userMessage("hi"))
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.chatCommands = { _ in ChatCommands() }
            $0.daemon.watch = { _, _ in
                AsyncThrowingStream { continuation in
                    continuation.yield(.event(said))
                    continuation.yield(.historyLoaded)
                    continuation.yield(.event(said))
                    continuation.yield(.historyLoaded)
                }
            }
            $0.daemon.markSeen = { _, _ in }
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.receivedAll)
        XCTAssertTrue(store.state.historyLoaded)
        XCTAssertEqual(store.state.events.map(\.id), ["e1"])
        await store.skipInFlightEffects()
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
        XCTAssertEqual(state.highlightedItem, "e2")
        XCTAssertNil(state.forkPointAfterHighlight)
    }

    func testAHighlightedOutputOfACallIdUsedAgainIsShownOnItsOwnTurnsRow() {
        let session = Session(id: "s", name: "s", backend: .scripted, workingDirectory: "/work")
        let now = Date(timeIntervalSince1970: 0)
        let events = [
            SessionEvent(id: "e1", time: now, kind: .toolStarted(callID: "c1", tool: "shell", argumentsJSON: "{}")),
            SessionEvent(id: "e2", time: now, kind: .toolCompleted(callID: "c1", success: true, output: "one")),
            SessionEvent(id: "e3", time: now, kind: .turnEnded(aborted: false)),
            SessionEvent(id: "e4", time: now, kind: .toolStarted(callID: "c1", tool: "shell", argumentsJSON: "{}")),
            SessionEvent(id: "e5", time: now, kind: .toolCompleted(callID: "c1", success: true, output: "two")),
        ]
        var state = SessionFeature.State(session: session, highlighted: "e2")
        state.events = IdentifiedArray(uniqueElements: events)
        XCTAssertEqual(state.highlightedItem, "e1")
        state.highlighted = "e5"
        XCTAssertEqual(state.highlightedItem, "e4")
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

    func testPuttingAMoveOffFromTheSessionDismissesItsRequestOnce() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = SessionFeature.State(session: session)
        state.request = AgentRequest(
            id: "r1", sessionID: "s1", kind: .move(item: "i1", status: .inReview), text: "Done",
            created: Date(timeIntervalSince1970: 0))
        // Exhaustive: an answer handed on that isn't received here fails.
        let store = TestStore(initialState: state) { SessionFeature() }

        await store.send(.movePutOff) {
            $0.answeringRequest = true
            $0.requestAnswer = .puttingOff
        }
        await store.receive(\.delegate.answerRequest)
        // Pressed again before the answer lands: nothing more is handed on.
        await store.send(.movePutOff)
        await store.send(.moveApproved)
    }

    func testAnEarlierMessagesEditorTakesTheCaretOnceAndCancellingGivesItToTheBox() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = SessionFeature.State(session: session)
        state.events = [event("e1", .userMessage("one"))]
        let store = store(state)

        await store.send(.editTapped("e1"))
        XCTAssertTrue(store.state.editCaret)
        await store.send(.editCaretTaken)
        XCTAssertFalse(store.state.editCaret, "a row scrolled back into view takes nothing")
        XCTAssertEqual(store.state.editing, "e1")

        let before = store.state.boxFocus
        await store.send(.editCancelled)
        XCTAssertNil(store.state.editing)
        XCTAssertNotEqual(store.state.boxFocus, before)
    }

    func testShowingProposalsScrollsToTheFirstOneWaiting() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = SessionFeature.State(session: session)
        state.proposals.proposals = [
            Proposal(id: "p1", title: "Done", state: .declined),
            Proposal(id: "p2", title: "Log in"),
            Proposal(id: "p3", title: "Log out"),
        ]
        let now = Date(timeIntervalSince1970: 0)
        state.events = IdentifiedArray(
            uniqueElements: ["p1", "p2", "p3"].enumerated().map { index, call in
                SessionEvent(
                    id: "e\(index)", time: now,
                    kind: .toolStarted(callID: call, tool: "humata__propose_task", argumentsJSON: "{}"))
            })
        let store = store(state)
        await store.send(.showProposalsTapped) {
            $0.scrollTarget = "e1"
        }
        await store.send(.scrolled) {
            $0.scrollTarget = nil
        }
    }

    /// Show goes to the waiting proposal that comes first in the chat,
    /// however they're listed, and goes there again each time it's pressed.
    func testShowingProposalsGoesToTheEarliestInTheChatEachTime() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = SessionFeature.State(session: session)
        // Listed newest first; the call `again` was also made in an
        // earlier turn, where it proposed nothing.
        state.proposals.proposals = [
            Proposal(id: "late", title: "Log out"),
            Proposal(id: "again", title: "Log in"),
        ]
        let now = Date(timeIntervalSince1970: 0)
        state.events = IdentifiedArray(
            uniqueElements: ["again", "again", "late"].enumerated().map { index, call in
                SessionEvent(
                    id: "e\(index)", time: now,
                    kind: .toolStarted(callID: call, tool: "humata__propose_task", argumentsJSON: "{}"))
            })
        let store = store(state)
        for _ in 0..<2 {
            await store.send(.showProposalsTapped) {
                $0.scrollTarget = "e1"
            }
            await store.send(.scrolled) {
                $0.scrollTarget = nil
            }
        }
    }

    /// Waiting proposals with no row in the chat aren't gone to: they're
    /// listed again, so the strip says what waits now.
    func testShowingProposalsWithNoRowInTheChatListsThemAgain() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = SessionFeature.State(session: session)
        state.proposals.proposals = [Proposal(id: "gone", title: "Log in")]
        let store = TestStore(initialState: state) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.markSeen = { _, _ in }
            $0.daemon.proposals = { _ in [] }
        }
        store.exhaustivity = .off
        await store.send(.showProposalsTapped)
        await store.receive(\.proposals.listed)
        XCTAssertNil(store.state.scrollTarget)
        XCTAssertEqual(store.state.pendingProposals, [])
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

    func testAHistoryComesAsOneBatchHoweverItsEventsArrive() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let history = [
            event("e1", .userMessage("one")), event("e2", .turnStarted), event("e3", .assistantMessage("two")),
        ]
        let (stream, continuation) = AsyncThrowingStream<WatchItem, any Error>.makeStream()
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.chatCommands = { _ in ChatCommands() }
            $0.daemon.watch = { _, _ in stream }
            $0.daemon.markSeen = { _, _ in }
        }
        store.exhaustivity = .off

        await store.send(.task)
        // One at a time, as a host sends them.
        for one in history {
            continuation.yield(.event(one))
            await Task.megaYield()
        }
        XCTAssertFalse(store.state.historyLoaded)
        XCTAssertTrue(store.state.events.isEmpty)

        continuation.yield(.historyLoaded)
        await store.receive(\.receivedAll)
        XCTAssertTrue(store.state.historyLoaded)
        XCTAssertEqual(store.state.events.map(\.id), ["e1", "e2", "e3"])

        // What comes after is new, and shows as it comes.
        continuation.yield(.event(event("e4", .turnEnded(aborted: false))))
        await store.receive(\.receivedAll)
        XCTAssertEqual(store.state.events.map(\.id), ["e1", "e2", "e3", "e4"])
        await store.skipInFlightEffects()
    }

    func testAWatchThatEndsBeforeItsHistoryHasAllComeShowsWhatCame() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let kept = [event("e1", .userMessage("hi")), event("e2", .assistantMessage("hello"))]
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.chatCommands = { _ in ChatCommands() }
            $0.daemon.watch = { _, _ in
                AsyncThrowingStream { continuation in
                    for event in kept { continuation.yield(.event(event)) }
                    continuation.finish()
                }
            }
            $0.daemon.markSeen = { _, _ in }
            $0.continuousClock = TestClock()
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.receivedAll)
        await store.receive(\.watchEnded)
        XCTAssertTrue(store.state.historyLoaded)
        XCTAssertEqual(store.state.events.map(\.id), ["e1", "e2"])
        await store.skipInFlightEffects()
    }

    /// An agent can use one id for calls in different turns; the chat's
    /// lazy list leaves a row blank when two rows share an id.
    func testToolCallsWithAnIdUsedAgainInALaterTurnAreTheirOwnRowsAndGetTheirOwnOutput() {
        let events = [
            event("e1", .turnStarted),
            event("e2", .toolStarted(callID: "call-0", tool: "shell", argumentsJSON: #"{"command":"one"}"#)),
            event("e3", .toolCompleted(callID: "call-0", success: true, output: "first")),
            event("e4", .turnEnded(aborted: false)),
            event("e5", .turnStarted),
            event("e6", .toolStarted(callID: "call-0", tool: "shell", argumentsJSON: #"{"command":"two"}"#)),
            event("e7", .toolCompleted(callID: "call-0", success: false, output: "second")),
            event("e8", .turnEnded(aborted: false)),
        ]
        let items = ChatItem.items(events)
        XCTAssertEqual(Set(items.map(\.id)).count, items.count)
        let calls = items.compactMap { item -> ToolCall? in
            if case .tool(let call) = item { call } else { nil }
        }
        XCTAssertEqual(calls.map(\.output), ["first", "second"])
        XCTAssertEqual(calls.map(\.succeeded), [true, false])
        XCTAssertEqual(calls.map(\.callID), ["call-0", "call-0"])
    }

    /// The chat's tool calls for `calls`, each a tool and its arguments.
    private func toolCalls(_ calls: [(tool: String, arguments: [String: Any])]) throws -> [ToolCall] {
        var events: [SessionEvent] = []
        for (index, call) in calls.enumerated() {
            let data = try JSONSerialization.data(withJSONObject: call.arguments)
            // swiftlint:disable:next optional_data_string_conversion
            let json = String(decoding: data, as: UTF8.self)
            events.append(
                event("e\(index)", .toolStarted(callID: "c\(index)", tool: call.tool, argumentsJSON: json)))
        }
        return ChatItem.items(events).compactMap { item -> ToolCall? in
            if case .tool(let call) = item { call } else { nil }
        }
    }

    func testALongWriteKeepsItsFirstLinesAndSaysHowManyItHas() throws {
        let lines = (1...1_000).map { "line \($0)" }
        let wide = String(repeating: "x", count: 4_000)
        let calls = try toolCalls([
            (tool: "Write", arguments: ["file_path": "big.txt", "content": lines.joined(separator: "\n")]),
            (tool: "write_file", arguments: ["path": "min.js", "contents": wide]),
        ])
        let big = try XCTUnwrap(calls.first?.change)
        XCTAssertEqual(big.removedCount, 0)
        XCTAssertEqual(big.addedCount, 1_000)
        XCTAssertEqual(big.added, Array(lines.prefix(300)))
        XCTAssertEqual(calls.first?.subject, "big.txt")

        let minified = try XCTUnwrap(calls.last?.change)
        XCTAssertEqual(minified.addedCount, 1)
        XCTAssertLessThan(minified.added.first?.count ?? .max, 1_000)
        XCTAssertEqual(minified.added.first.map { wide.hasPrefix($0.dropLast()) }, true)
    }

    func testAnEditNamesItsFileAndShowsWhatItTakesOutAndPutsIn() throws {
        let removed = (1...200).map { "old \($0)" }
        let added = (1...200).map { "new \($0)" }
        let edit: [String: Any] = [
            "file_path": "a.swift", "old_string": "let a = 1", "new_string": "let a = 2\nlet b = 3",
        ]
        let edits: [[String: String]] = [
            ["old_string": removed.joined(separator: "\n"), "new_string": ""],
            ["old_string": "", "new_string": added.joined(separator: "\n")],
        ]
        let multiEdit: [String: Any] = ["file_path": "b.swift", "edits": edits]
        let calls = try toolCalls([
            (tool: "Edit", arguments: edit), (tool: "MultiEdit", arguments: multiEdit),
            (tool: "shell", arguments: ["command": "ls"]),
        ])
        XCTAssertEqual(calls.map(\.subject), ["a.swift", "b.swift", "ls"])
        XCTAssertEqual(calls[0].change, ToolCall.Change(removed: ["let a = 1"], added: ["let a = 2", "let b = 3"]))
        XCTAssertNil(calls[2].change)

        // The lines taken out come first in what's kept.
        let many = try XCTUnwrap(calls[1].change)
        XCTAssertEqual(many.removedCount, 200)
        XCTAssertEqual(many.addedCount, 200)
        XCTAssertEqual(many.removed, removed)
        XCTAssertEqual(many.added, Array(added.prefix(100)))
    }

    func testSendingAFailedMessageAgainSendsItsImagesToo() async throws {
        let picture = ImageAttachment(mediaType: "image/png", data: try png())
        let sent = LockIsolated<[String]>([])
        let store = queueStore("retry-images") {
            $0.daemon.readImage = { _, _ in picture }
            $0.daemon.send = { _, text, _ in sent.withValue { $0.append("text only: \(text)") } }
            $0.daemon.sendImages = { _, text, images, _ in
                sent.withValue { $0.append("\(text) with \(images.count), same: \(images == [picture])") }
            }
        }
        await store.send(.received(event("e1", .userMessage("what is this", images: ["a.png"]))))
        await store.send(.received(event("e2", .turnStarted)))
        await store.send(.received(event("e3", .failed("the model server went away"))))
        await store.send(.received(event("e4", .turnEnded(aborted: false))))
        await store.send(.retryTapped)
        await store.finish()
        XCTAssertEqual(sent.value, ["what is this with 1, same: true"])
    }

    func testAMessageRewoundToComesBackWithItsImagesThatUndoTakesOut() async throws {
        let picture = ImageAttachment(mediaType: "image/png", data: try png())
        let store = queueStore("rewound-images") {
            $0.uuid = .incrementing
            $0.daemon.readImage = { _, _ in picture }
            $0.daemon.rewind = { _, _, _ in }
        }
        await store.send(\.binding.readsImages, true)
        await store.send(.received(event("e1", .userMessage("what is this", images: ["a.png"]))))
        await store.send(.rewindTapped("e1"))
        await store.send(.rewindDialog(.presented(.chat("e1"))))
        await store.receive(\.sentImagesRead)
        await store.receive(\.rewound)
        XCTAssertEqual(store.state.draft, "what is this")
        XCTAssertEqual(store.state.images.map(\.name), ["a.png"])
        XCTAssertEqual(store.state.images.map(\.image), [picture])
        XCTAssertTrue(store.state.canSend)

        await store.send(.rewindUndone)
        XCTAssertEqual(store.state.images, [])
        XCTAssertEqual(store.state.draft, "")
    }

    func testStopAndEditPutsTheMessagesImagesBackToo() async throws {
        let picture = ImageAttachment(mediaType: "image/png", data: try png())
        let store = queueStore("stop-and-edit-images") {
            $0.uuid = .incrementing
            $0.daemon.readImage = { _, _ in picture }
            $0.daemon.abort = { _ in }
        }
        await store.send(\.binding.readsImages, true)
        await store.send(.received(event("e1", .userMessage("what is this", images: ["a.png"]))))
        await store.send(.received(event("e2", .turnStarted)))
        await store.send(.stopAndEditTapped)
        await store.receive(\.sentImagesRead)
        XCTAssertEqual(store.state.draft, "what is this")
        XCTAssertEqual(store.state.images.map(\.image), [picture])
    }

    func testAVeryLongMessageRewoundToComesBackAsAChipThatUndoTakesOut() async {
        let store = queueStore("rewound-long") { $0.uuid = .incrementing }
        let long = String(repeating: "A pasted log line that goes on for a while.\n", count: 7_000)
        XCTAssertGreaterThan(long.utf8.count, 300_000)
        await store.send(\.binding.draft, "typed since")
        await store.send(.rewound(long))
        XCTAssertEqual(store.state.draft, "typed since")
        XCTAssertEqual(store.state.pastes.map(\.text), [long])
        XCTAssertTrue(store.state.canSend)

        await store.send(.rewindUndone)
        XCTAssertEqual(store.state.pastes, [])
        XCTAssertEqual(store.state.draft, "typed since")
    }

    func testAMessageRewoundToGoesInTheBoxToEditHoweverManyLinesItHas() async {
        let store = queueStore("rewound-short") { $0.uuid = .incrementing }
        let prompt = (1...60).map { "Step \($0): check the fixture parses." }.joined(separator: "\n")
        await store.send(.rewound(prompt))
        XCTAssertEqual(store.state.draft, prompt)
        XCTAssertEqual(store.state.pastes, [])

        await store.send(.rewindUndone)
        XCTAssertEqual(store.state.draft, "")
    }

    func testAVeryLongMessageStoppedToEditComesBackAsAChip() async {
        let stopped = LockIsolated(0)
        let store = queueStore("stop-and-edit-long") {
            $0.daemon.abort = { _ in stopped.withValue { $0 += 1 } }
            $0.uuid = .incrementing
        }
        let long = String(repeating: "x", count: 50_000)
        await store.send(.received(event("e1", .userMessage(long))))
        await store.send(.received(event("e2", .turnStarted)))
        await store.send(.stopAndEditTapped)
        // The chip leaves the box empty, so the button is still there.
        await store.send(.stopAndEditTapped)
        await store.finish()
        XCTAssertEqual(stopped.value, 1)
        XCTAssertEqual(store.state.draft, "")
        XCTAssertEqual(store.state.pastes.map(\.text), [long])
    }

    private func readyImage(_ id: String) -> DraftImage {
        DraftImage(id: id, name: "shot.png", image: ImageAttachment(mediaType: "image/png", data: Data([1])))
    }

    func testAModelListThatCantBeReadLeavesTheBoxesImagesAndWhatWasKnown() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(
            StartRequest(backend: .local, workingDirectory: "/work", name: "local", model: "qwen3:8b"))
        var state = SessionFeature.State(session: session)
        state.readsImages = true
        state.images = [readyImage("i1")]
        let listed = LockIsolated(false)
        let store = TestStore(initialState: state) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.models = { _, _, _ in
                listed.setValue(true)
                throw DaemonError.disconnected
            }
        }
        store.exhaustivity = .off

        let watching = await store.send(.task)
        await store.receive(\.readsImagesUnknown, timeout: .seconds(1))
        XCTAssertTrue(listed.value)
        XCTAssertEqual(store.state.images.map(\.id), ["i1"])
        XCTAssertEqual(store.state.readsImages, true)
        XCTAssertNil(store.state.alert)
        XCTAssertNil(store.state.imagesCheckFailure, "what was known still stands, so nothing waits")
        XCTAssertTrue(store.state.canSend)
        await watching.cancel()
    }

    func testImagesWaitingOnAModelListThatCantBeReadSayWhyAndGoOnceItIsRead() async {
        let session = Session(
            id: "images-wait", name: "local", backend: .local, workingDirectory: "/work", model: "qwen3:8b")
        var state = SessionFeature.State(session: session)
        state.images = [readyImage("i1")]
        XCTAssertNil(state.readsImages)
        let fails = LockIsolated(true)
        let store = TestStore(initialState: state) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.models = { _, _, _ in
                if fails.value { throw DaemonError.disconnected }
                return [ModelChoice(id: "qwen3:8b", name: "Qwen 3", readsImages: true)]
            }
        }
        store.exhaustivity = .off

        await store.send(.readsImagesAsked)
        await store.receive(\.readsImagesUnknown, timeout: .seconds(1))
        XCTAssertEqual(store.state.images.map(\.id), ["i1"])
        XCTAssertNotNil(store.state.imagesCheckFailure)
        XCTAssertNil(store.state.readsImages)
        XCTAssertFalse(store.state.canSend)

        // Try Again, with the list readable now.
        fails.setValue(false)
        await store.send(.readsImagesAsked)
        XCTAssertNil(store.state.imagesCheckFailure, "it's being looked for again")
        await store.receive(\.readsImagesFound, timeout: .seconds(1))
        XCTAssertNil(store.state.imagesCheckFailure)
        XCTAssertEqual(store.state.images.map(\.id), ["i1"])
        XCTAssertTrue(store.state.canSend)
    }

    func testAModelListThatSaysTheModelCantSeeImagesTakesTheBoxesImageOut() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(
            StartRequest(backend: .local, workingDirectory: "/work", name: "local", model: "qwen3:8b"))
        daemon.setModels([ModelChoice(id: "qwen3:8b", name: "Qwen 3", readsImages: false)], for: .local)
        var state = SessionFeature.State(session: session)
        state.images = [readyImage("i1")]
        let store = TestStore(initialState: state) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        let watching = await store.send(.task)
        await store.receive(\.readsImagesFound, timeout: .seconds(1))
        XCTAssertTrue(store.state.images.isEmpty)
        XCTAssertEqual(store.state.readsImages, false)
        XCTAssertNotNil(store.state.alert, "it says why")
        await watching.cancel()
    }

    func testAWatchThatStartsOverDoesntListTheModelsAgainOnceTheAnswerIsKnown() async throws {
        let session = Session(
            id: "models-once", name: "local", backend: .local, workingDirectory: "/work", model: "qwen3:8b")
        let listed = LockIsolated(0)
        let clock = TestClock()
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.chatCommands = { _ in ChatCommands() }
            $0.daemon.models = { _, _, _ in
                listed.withValue { $0 += 1 }
                return [ModelChoice(id: "qwen3:8b", name: "Qwen 3", readsImages: true)]
            }
            // The connection drops as soon as the history has come.
            $0.daemon.watch = { _, _ in
                AsyncThrowingStream { continuation in
                    continuation.yield(.historyLoaded)
                    continuation.finish(throwing: DaemonError.disconnected)
                }
            }
            $0.daemon.markSeen = { _, _ in }
            $0.continuousClock = clock
        }
        store.exhaustivity = .off

        await store.send(.task)
        await until(store) { $0.readsImages == true && $0.disconnected }
        XCTAssertEqual(listed.value, 1)

        await clock.advance(by: .seconds(1))
        await store.receive(\.task)
        await store.receive(\.watchEnded)
        XCTAssertEqual(listed.value, 1, "the watch started over without asking again")
        XCTAssertEqual(store.state.readsImages, true)
        await store.skipInFlightEffects()
    }

    func testEveryForkOfAReadOnlySessionIsAskedForChatOnlyAndNoChatUnderItForItsOwnCopy() async {
        var session = Session(id: "side-forks", name: "side", backend: .scripted, workingDirectory: "/work")
        session.readOnly = true
        var made = session
        made.id = "side-forks-fork"
        let fork = made
        let chatOnly = LockIsolated<[Bool]>([])
        let children = LockIsolated<[ChildChat]>([])
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.fork = { _, _, _, only in
                chatOnly.withValue { $0.append(only) }
                return fork
            }
            $0.daemon.newChild = { _, _, kind in
                children.withValue { $0.append(kind) }
                return fork
            }
            $0.daemon.send = { _, _, _ in }
            $0.daemon.markSeen = { _, _ in }
        }
        store.exhaustivity = .off

        // Fork With the Code, as the Session menu sends it.
        await store.send(.forkTapped(before: nil, chatOnly: false))
        await store.receive(\.delegate.openSession, timeout: .seconds(1))
        XCTAssertEqual(chatOnly.value, [true])

        // "/fork", with the space Tab leaves after it.
        await store.send(\.binding.draft, "/fork ")
        await store.send(.sendTapped(.enqueue))
        await store.receive(\.delegate.openSession, timeout: .seconds(1))
        XCTAssertEqual(chatOnly.value, [true, true])

        await store.send(.received(event("e1", .userMessage("hello"))))
        await store.send(.editTapped("e1"))
        await store.send(\.binding.edited, "hello again")
        await store.send(.editSent)
        await store.receive(\.delegate.openSession, timeout: .seconds(1))
        XCTAssertEqual(chatOnly.value, [true, true, true])

        await store.send(.childTapped(.empty(ownCopy: true), message: ""))
        await store.receive(\.delegate.openSession, timeout: .seconds(1))
        XCTAssertEqual(children.value, [.empty(ownCopy: false)])
    }

    func testAnImageAddedBeforeTheModelIsKnownWaitsAndComesOutIfItCantBeSeen() async throws {
        let session = Session(id: "images-unknown", name: "local", backend: .local, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off

        await store.send(.imagePasted(try png()))
        await store.receive(\.imagePrepared)
        XCTAssertEqual(store.state.images.count, 1)
        XCTAssertNil(store.state.alert)
        XCTAssertFalse(store.state.canSend, "it waits until the model is known to see it")

        await store.send(.readsImagesFound(false))
        XCTAssertTrue(store.state.images.isEmpty)
        XCTAssertNotNil(store.state.alert, "it says why")
    }

    func testAnImageAddedBeforeTheModelIsKnownGoesOnceItCanBeSeen() async throws {
        let session = Session(id: "images-known", name: "local", backend: .local, workingDirectory: "/work")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off

        await store.send(.imagePasted(try png()))
        await store.receive(\.imagePrepared)
        await store.send(.readsImagesFound(true))
        XCTAssertEqual(store.state.images.count, 1)
        XCTAssertNil(store.state.alert)
        XCTAssertTrue(store.state.canSend)
    }

    func testImagesReadyToSendAreKeptWhenASessionIsLeft() {
        let session = Session(id: "images-left", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = SessionFeature.State(session: session)
        state.images = [
            DraftImage(id: "i1", name: "ready.png", image: ImageAttachment(mediaType: "image/png", data: Data([1]))),
            DraftImage(id: "i2", name: "still-being-read.png"),
        ]

        var opened = SessionFeature.State(session: session)
        opened.left = state.left
        // One still being read would never be ready in the session opened
        // again, so it isn't kept.
        XCTAssertEqual(opened.images.map(\.id), ["i1"])
        XCTAssertEqual(opened.images.first?.image, state.images.first?.image)
    }

    func testStopPressedTwiceStopsTheTurnOnce() async {
        let stopped = LockIsolated(0)
        let store = queueStore("stop-twice") { $0.daemon.abort = { _ in stopped.withValue { $0 += 1 } } }
        await store.send(.received(event("e1", .userMessage("write the notes"))))
        await store.send(.received(event("e2", .turnStarted)))
        await store.send(.abortTapped)
        XCTAssertTrue(store.state.stopping)
        await store.send(.abortTapped)
        await store.send(.stopAndEditTapped)
        await store.finish()
        XCTAssertEqual(stopped.value, 1)
        XCTAssertEqual(store.state.draft, "")

        await store.send(.received(event("e3", .turnEnded(aborted: true))))
        XCTAssertFalse(store.state.stopping)
    }

    func testStopWorksAgainOnceTheStoppedTurnEndsThoughTheNextHasStarted() async {
        let stopped = LockIsolated(0)
        let store = queueStore("stop-next") { $0.daemon.abort = { _ in stopped.withValue { $0 += 1 } } }
        await store.send(.received(event("e1", .turnStarted)))
        await store.send(.abortTapped)
        // The turn ends and the queued message's starts, together.
        await store.send(.receivedAll([event("e2", .turnEnded(aborted: true)), event("e3", .turnStarted)]))
        XCTAssertFalse(store.state.stopping)
        XCTAssertTrue(store.state.isWorking)
        await store.send(.abortTapped)
        await store.finish()
        XCTAssertEqual(stopped.value, 2)
    }

    func testAStopThatFailedCanBePressedAgain() async {
        let store = queueStore("stop-failed") { $0.daemon.abort = { _ in throw DaemonError.disconnected } }
        await store.send(.received(event("e1", .turnStarted)))
        await store.send(.abortTapped)
        await store.receive(\.failed)
        XCTAssertFalse(store.state.stopping)
        XCTAssertNotNil(store.state.alert)
    }
}

extension ChatItem {
    fileprivate func isPerson(_ text: String) -> Bool {
        if case .person(_, text, _) = self { true } else { false }
    }
}

extension SessionFeatureTests {
    func testAClaudeSessionTakesImagesBeforeItsChatOpens() {
        let claude = Session(id: "images-claude", name: "first", backend: .claude, workingDirectory: "/work")
        XCTAssertTrue(SessionFeature.State(session: claude).takesImages)
        let local = Session(id: "images-local", name: "first", backend: .local, workingDirectory: "/work")
        XCTAssertFalse(SessionFeature.State(session: local).takesImages, "not known until its models are listed")
    }

    func testACopilotSessionOpenedAgainTakesImagesAtOnceWhenItsModelDidBefore() async {
        let session = Session(
            id: "images-again", name: "first", backend: .copilot, workingDirectory: "/work", model: "gpt-5")
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        }
        store.exhaustivity = .off
        XCTAssertFalse(store.state.takesImages)
        await store.send(.readsImagesFound(true))
        XCTAssertTrue(store.state.takesImages)

        XCTAssertTrue(SessionFeature.State(session: session).takesImages, "opened again, before it's asked")
        // What was found for one model says nothing of another.
        var other = session
        other.model = "small"
        XCTAssertFalse(SessionFeature.State(session: other).takesImages)
    }

    /// A session whose first message, `e1`, has `images`, over a daemon
    /// that notes each fork and each message sent in `calls`.
    private func editStore(
        images: [String], calls: LockIsolated<[String]>,
        _ dependencies: @escaping (inout DependencyValues) -> Void = { _ in }
    ) -> TestStoreOf<SessionFeature> {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let fork = Session(id: "s2", name: "first edit", backend: .scripted, workingDirectory: "/work")
        var state = SessionFeature.State(session: session)
        state.events = [event("e1", .userMessage("what is this", images: images))]
        let store = TestStore(initialState: state) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.markSeen = { _, _ in }
            $0.daemon.chatCommands = { _ in ChatCommands() }
            $0.daemon.fork = { _, _, _, _ in
                calls.withValue { $0.append("fork") }
                return fork
            }
            $0.daemon.send = { session, text, _ in calls.withValue { $0.append("\(session) text: \(text)") } }
            $0.daemon.sendImages = { session, text, images, _ in
                calls.withValue { $0.append("\(session) \(text) with \(images.map(\.mediaType))") }
            }
            dependencies(&$0)
        }
        store.exhaustivity = .off
        return store
    }

    func testAnEditedMessagesImagesGoToTheFork() async {
        let calls = LockIsolated<[String]>([])
        let store = editStore(images: ["a.png", "b.jpg"], calls: calls) {
            $0.daemon.readImage = { _, name in
                ImageAttachment(mediaType: name == "a.png" ? "image/png" : "image/jpeg", data: Data([1]))
            }
        }
        await store.send(.editTapped("e1"))
        await store.send(.binding(.set(\.edited, "and this?")))
        await store.send(.editSent)
        await store.receive(\.delegate.openSession)
        XCTAssertEqual(calls.value, ["fork", #"s2 and this? with ["image/png", "image/jpeg"]"#])
        XCTAssertNil(store.state.editing)
        XCTAssertEqual(store.state.editedImages, [])
    }

    func testAnImageThatCantBeReadLeavesTheEditOpenAndMakesNoFork() async {
        let calls = LockIsolated<[String]>([])
        let store = editStore(images: ["a.png"], calls: calls) {
            $0.daemon.readImage = { _, _ in throw DaemonError.disconnected }
        }
        await store.send(.editTapped("e1"))
        await store.send(.binding(.set(\.edited, "and this?")))
        await store.send(.editSent)
        await store.receive(\.failed)
        XCTAssertEqual(calls.value, [])
        XCTAssertEqual(store.state.editing, "e1")
        XCTAssertEqual(store.state.edited, "and this?")
        XCTAssertEqual(store.state.editedImages, ["a.png"])
        XCTAssertFalse(store.state.forking)
    }

    func testAnEditWithImagesThatDidntReachItsForkStillOpensTheFork() async {
        let calls = LockIsolated<[String]>([])
        let store = editStore(images: ["a.png"], calls: calls) {
            $0.daemon.readImage = { _, _ in ImageAttachment(mediaType: "image/png", data: Data([1])) }
            $0.daemon.sendImages = { _, _, _, _ in throw DaemonError.disconnected }
        }
        await store.send(.editTapped("e1"))
        await store.send(.binding(.set(\.edited, "and this?")))
        await store.send(.editSent)
        await store.receive(\.forkSendFailed)
        XCTAssertEqual(calls.value, ["fork"])
        XCTAssertNotNil(store.state.alert)
        XCTAssertNil(store.state.editing)
        XCTAssertEqual(store.state.editedImages, [])

        await store.send(.alert(.dismiss))
        await store.receive(\.delegate.openSession)
        // The fork's message box has the edit's text.
        let fork = Session(id: "s2", name: "first edit", backend: .scripted, workingDirectory: "/work")
        XCTAssertTrue(SessionFeature.State(session: fork).draft.hasSuffix("and this?"))
    }

    func testAnEditedMessageWithNoImagesIsSentAsText() async {
        let calls = LockIsolated<[String]>([])
        let store = editStore(images: [], calls: calls)
        await store.send(.editTapped("e1"))
        await store.send(.binding(.set(\.edited, "and this?")))
        await store.send(.editSent)
        await store.receive(\.delegate.openSession)
        XCTAssertEqual(calls.value, ["fork", "s2 text: and this?"])
    }
}
