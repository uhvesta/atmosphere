import ComposableArchitecture
import HumataDaemonClient
import HumataSession
import XCTest

/// Humata's own / commands: /rewind picks where to cut the chat back to, a
/// compaction included, and /compact has the agent sum the conversation up.
@MainActor
final class ChatCommandTests: XCTestCase {
    private func store(_ daemon: InMemoryDaemon, _ session: Session) -> TestStoreOf<SessionFeature> {
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.continuousClock = ImmediateClock()
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_000))
        }
        store.exhaustivity = .off
        return store
    }

    /// A store fed its events by hand, with no daemon behind it but what
    /// `dependencies` sets.
    private func fed(
        _ id: String, _ dependencies: @escaping (inout DependencyValues) -> Void = { _ in }
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

    private func event(_ id: String, _ kind: SessionEvent.Kind) -> SessionEvent {
        SessionEvent(id: id, time: Date(timeIntervalSince1970: 0), kind: kind)
    }

    private func started(_ daemon: InMemoryDaemon, backend: Backend = .scripted) async throws -> Session {
        try await daemon.client.start(StartRequest(backend: backend, workingDirectory: "/work", name: "first"))
    }

    private func skilled(_ names: String...) -> InMemoryDaemon {
        let daemon = InMemoryDaemon()
        daemon.setSkills(
            SkillList(skills: names.enumerated().map { FoundSkill(name: $1, origin: .claudeUser, hash: "\($0)") }))
        return daemon
    }

    /// Where `kind` is in the session's history; nil if it isn't.
    private func index(of kind: SessionEvent.Kind, in daemon: InMemoryDaemon, _ session: Session) -> Int? {
        daemon.history(session.id).firstIndex { $0.kind == kind }
    }

    private func compactions(_ daemon: InMemoryDaemon, _ session: Session) -> [SessionEvent] {
        daemon.history(session.id).filter { event in
            switch event.kind {
            case .compacting, .compacted: true
            default: false
            }
        }
    }

    private func messages(_ picker: SessionFeature.RewindPicker?) -> [String] {
        (picker?.points ?? []).map { point in
            switch point.kind {
            case .message(let text): text
            case .compaction: "compaction"
            }
        }
    }

    // MARK: The / list

    func testTheListStartsWithHumatasCommandsAndOneReplacesASkillOfItsName() async throws {
        let daemon = skilled("compact", "release")
        let store = store(daemon, try await started(daemon, backend: .claude))

        await store.send(\.binding.draft, "/")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        XCTAssertEqual(
            store.state.slashChoices.map(\.id), ["rewind", "compact", "new-here", "fork", "side", "release"])
        XCTAssertEqual(store.state.slashChoices.map(\.builtIn), [.rewind, .compact, .newHere, .fork, .side, nil])
    }

    func testAnAgentThatCantCompactIsNotOfferedItAndTheCommandGoesToItAsAMessage() async throws {
        let daemon = skilled("release")
        daemon.setChatCommands(ChatCommands())
        let session = try await started(daemon, backend: .claude)
        try await daemon.client.send(session.id, "one", .enqueue)
        let store = store(daemon, session)

        await store.send(\.binding.draft, "/")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        XCTAssertEqual(store.state.slashChoices.map(\.id), ["rewind", "new-here", "fork", "release"])

        await store.send(\.binding.draft, "/compact the notes")
        await store.send(.sendTapped(.enqueue))
        var sent: String?
        await store.receive({ action in
            guard case .delegate(.send(let outgoing)) = action else { return false }
            sent = outgoing.text
            return true
        })
        XCTAssertEqual(sent, "/compact the notes")
        XCTAssertEqual(compactions(daemon, session), [])
    }

    func testSkillsAreStillListedWhenTheHostCantSayWhichCommandsItTakes() async throws {
        let daemon = skilled("release")
        daemon.setChatCommands(nil)
        let store = store(daemon, try await started(daemon, backend: .claude))

        await store.send(\.binding.draft, "/")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        XCTAssertEqual(store.state.slashChoices.map(\.id), ["rewind", "new-here", "fork", "release"])
        XCTAssertNil(store.state.slashFailure)
    }

    func testAnArchivedSessionIsOfferedNoneOfHumatasCommands() {
        var session = Session(id: "s1", name: "chat", backend: .claude, workingDirectory: "/work")
        let commands = ChatCommands(compact: true, compactTakesFocus: true)
        XCTAssertEqual(
            SlashChoice.builtIns(for: session, commands: commands).map(\.builtIn), [.rewind, .compact, .newHere, .fork])
        session.archived = true
        XCTAssertEqual(SlashChoice.builtIns(for: session, commands: commands), [])
    }

    func testOnlyHumatasCommandsAreReadFromADraft() {
        XCTAssertEqual(SlashChoice.command(in: "/rewind")?.command, .rewind)
        XCTAssertEqual(SlashChoice.command(in: "/compact")?.command, .compact)
        XCTAssertEqual(SlashChoice.command(in: "/compact")?.text, "")
        XCTAssertEqual(SlashChoice.command(in: "/compact  keep the API notes \n")?.text, "keep the API notes")
        XCTAssertNil(SlashChoice.command(in: "/compaction"))
        XCTAssertNil(SlashChoice.command(in: "/release now"))
        XCTAssertNil(SlashChoice.command(in: "compact"))
        XCTAssertNil(SlashChoice.command(in: "see /compact"))
    }

    // MARK: /rewind

    func testRewindOpensAPickerOfMessagesNewestFirstThatNarrowsMovesClosesAndPicks() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        for text in ["one", "two", "three"] { try await daemon.client.send(session.id, text, .enqueue) }
        let two = try XCTUnwrap(daemon.history(session.id).first { $0.kind == .userMessage("two") })
        let store = store(daemon, session)
        let watching = await store.send(.task)
        await store.receive(\.receivedAll, timeout: .seconds(1))

        await store.send(\.binding.draft, "/rewind")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        await store.send(.sendTapped(.enqueue))
        await store.receive(\.slashPicked)
        XCTAssertEqual(messages(store.state.rewindPicker), ["three", "two", "one"])
        XCTAssertEqual(store.state.draft, "")
        XCTAssertFalse(store.state.slashOpen)
        // The box handing back the empty draft leaves it open; typing in
        // the box is what closes it.
        await store.send(\.binding.draft, "")
        XCTAssertNotNil(store.state.rewindPicker)

        await store.send(.rewindPickerQueryChanged("tw"))
        XCTAssertEqual(messages(store.state.rewindPicker), ["two"])
        await store.send(.rewindPickerQueryChanged(""))
        XCTAssertEqual(messages(store.state.rewindPicker), ["three", "two", "one"])

        await store.send(.rewindPickerMoved(by: 1))
        XCTAssertEqual(store.state.rewindPicker?.selected, 1)
        await store.send(.rewindPickerMoved(by: -1))
        await store.send(.rewindPickerMoved(by: -1))
        XCTAssertEqual(store.state.rewindPicker?.selected, 2, "moving past the start wraps around")

        await store.send(.rewindPickerClosed)
        XCTAssertNil(store.state.rewindPicker)
        XCTAssertNil(store.state.rewindDialog)

        await store.send(\.binding.draft, "/rewind")
        await store.send(.sendTapped(.enqueue))
        await store.receive(\.slashPicked)
        XCTAssertNotNil(store.state.rewindPicker)
        await store.send(\.binding.draft, "x")
        XCTAssertNil(store.state.rewindPicker)
        await store.send(\.binding.draft, "")

        // What's typed after the command is what the picker looks for.
        await store.send(\.binding.draft, "/rewind two")
        await store.send(.sendTapped(.enqueue))
        XCTAssertEqual(messages(store.state.rewindPicker), ["two"])
        let picked = try XCTUnwrap(store.state.rewindPicker?.points.first?.id)
        XCTAssertEqual(picked, two.id)
        await store.send(.rewindPickerPicked(picked))
        await store.receive(\.rewindTapped)
        XCTAssertNil(store.state.rewindPicker)
        XCTAssertNotNil(store.state.rewindDialog)
        XCTAssertEqual(daemon.history(session.id).last?.kind, .idle, "nothing is rewound until it's confirmed")
        await watching.cancel()
    }

    func testRewindWithNothingToRewindToSaysSoInsteadOfOpeningAnEmptyPicker() async throws {
        let daemon = InMemoryDaemon()
        let store = store(daemon, try await started(daemon))
        let watching = await store.send(.task)
        await store.receive(\.receivedAll, timeout: .seconds(1))
        XCTAssertTrue(store.state.historyLoaded)

        await store.send(\.binding.draft, "/rewind")
        await store.send(.sendTapped(.enqueue))
        await store.receive(\.slashPicked)
        XCTAssertNil(store.state.rewindPicker)
        XCTAssertNotNil(store.state.alert)
        XCTAssertEqual(store.state.draft, "")
        await watching.cancel()
    }

    func testRewindWhileTheChatIsStillOpeningStaysInTheBoxUntilTheChatShows() async {
        let store = fed("rewind-opening")
        XCTAssertFalse(store.state.historyLoaded)

        await store.send(\.binding.draft, "/rewind one")
        await store.send(.sendTapped(.enqueue))
        XCTAssertNil(store.state.rewindPicker)
        XCTAssertNotNil(store.state.alert, "it says the chat isn't here yet")
        XCTAssertEqual(store.state.draft, "/rewind one")

        await store.send(.alert(.dismiss))
        await store.send(.receivedAll([event("e1", .userMessage("one"))]))
        await store.send(.sendTapped(.enqueue))
        XCTAssertEqual(messages(store.state.rewindPicker), ["one"])
        XCTAssertEqual(store.state.draft, "")
    }

    func testReturnInTheEmptyBoxUnderThePickerPicksTheHighlightedPoint() async {
        let store = fed("rewind-return")
        await store.send(
            .receivedAll([
                event("e1", .userMessage("one")), event("e2", .turnStarted), event("e3", .turnEnded(aborted: false)),
                event("e4", .userMessage("two")), event("e5", .turnStarted), event("e6", .turnEnded(aborted: false)),
            ]))
        await store.send(\.binding.draft, "/rewind ")
        await store.send(.sendTapped(.enqueue))
        XCTAssertEqual(messages(store.state.rewindPicker), ["two", "one"])
        await store.send(.rewindPickerMoved(by: 1))

        await store.send(.sendTapped(.enqueue))
        await store.receive(\.rewindPickerPicked)
        await store.receive(\.rewindTapped)
        XCTAssertNil(store.state.rewindPicker)
        XCTAssertEqual(rewindChoices(store), [.chat("e1")], "the one the arrows moved to")
    }

    /// What the rewind dialog showing offers, but Cancel.
    private func rewindChoices(_ store: TestStoreOf<SessionFeature>) -> [SessionFeature.Action.RewindChoice] {
        (store.state.rewindDialog?.buttons ?? []).compactMap(\.action.action)
    }

    func testRewindingFilesIsOfferedOnlyToASessionWithItsOwnCopyOfTheCode() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject(
            "App", [Repository(name: "app", path: "/code/app")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, name: "first", approvalMode: .allowEverything, projectID: project.id))
        try await daemon.client.send(session.id, "one", .enqueue)
        let shared = try await daemon.client.fork(session.id, nil, "chat only", true)
        let own = try await daemon.client.fork(session.id, nil, "with the code", false)
        let side = try await daemon.client.newChild(session.id, "side", .side)
        XCTAssertTrue(shared.sharesCode)
        XCTAssertFalse(own.sharesCode)

        func offered(_ session: Session) async throws -> [SessionFeature.Action.RewindChoice] {
            let store = store(daemon, session)
            let watching = await store.send(.task)
            await store.receive(\.receivedAll, timeout: .seconds(1))
            let message = try XCTUnwrap(daemon.history(session.id).first { $0.kind == .userMessage("one") })
            await store.send(.rewindTapped(message.id))
            await watching.cancel()
            return rewindChoices(store)
        }

        let one = try XCTUnwrap(daemon.history(session.id).first { $0.kind == .userMessage("one") }).id
        let rewinds = try await [offered(session), offered(own), offered(shared), offered(side)]
        XCTAssertEqual(rewinds[0], [.chat(one), .chatAndFiles(one)])
        XCTAssertEqual(rewinds[1].count, 2, "a fork with its own copy rewinds its files too")
        XCTAssertEqual(rewinds[2].count, 1, "a chat-only fork works in a copy that isn't its own")
        XCTAssertEqual(rewinds[3].count, 1, "a side chat only reads")
        for choices in rewinds[1...] {
            guard case .chat? = choices.first else { return XCTFail("The chat alone isn't offered first") }
        }
    }

    func testThePickerListsCompactionsAndLeavesOutASubagentsEvents() {
        var inner = [event("e3", .compacted), event("e4", .userMessage("inner"))]
        for index in inner.indices { inner[index].parentCallID = "c1" }
        let events =
            [event("e1", .userMessage("one")), event("e2", .compacted)] + inner
            + [event("e5", .assistantMessage("two"))]
        let points = RewindPoint.points(in: events)
        XCTAssertEqual(points.map(\.id), ["e2", "e1"])
        XCTAssertEqual(points.map(\.kind), [.compaction, .message("one")])
        XCTAssertEqual(RewindPoint.matching("compact", in: points).map(\.id), ["e2"])
        XCTAssertEqual(RewindPoint.matching("zzz", in: points), [])
    }

    func testRewindingToACompactionTakesItOutPutsNothingInTheBoxAndUndoBringsItBack() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        try await daemon.client.send(session.id, "one", .enqueue)
        let before = daemon.history(session.id)
        try await daemon.client.compact(session.id, "", false)
        try await daemon.client.send(session.id, "two", .enqueue)
        let whole = daemon.history(session.id)
        let compacted = try XCTUnwrap(whole.first { $0.kind == .compacted })
        let rewoundTo = LockIsolated<[String]>([])
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            let client = daemon.client
            $0.daemon = client
            $0.daemon.rewind = { session, event, files in
                rewoundTo.withValue { $0.append(event) }
                try await client.rewind(session, event, files)
            }
            $0.continuousClock = ImmediateClock()
        }
        store.exhaustivity = .off
        let watching = await store.send(.task)
        await store.receive(\.receivedAll, timeout: .seconds(1))

        await store.send(.rewindTapped(compacted.id))
        XCTAssertNotNil(store.state.rewindDialog)
        await store.send(.rewindDialog(.presented(.chat(compacted.id))))
        await store.receive(\.rewound, timeout: .seconds(1))
        XCTAssertEqual(rewoundTo.value, [compacted.id])
        XCTAssertEqual(store.state.draft, "")
        XCTAssertEqual(store.state.pastes, [])
        XCTAssertEqual(compactions(daemon, session), [])
        XCTAssertEqual(Array(daemon.history(session.id).dropLast()), before, "all of the turn that compacted goes")

        await store.send(.undoRewindTapped)
        await store.receive(\.rewindUndone, timeout: .seconds(1))
        XCTAssertEqual(Array(daemon.history(session.id).dropLast()), whole)
        XCTAssertEqual(store.state.draft, "")
        await watching.cancel()
    }

    func testOnlyAMessageOrACompactionOfTheSessionsOwnCanBeRewoundTo() async {
        let store = fed("rewind-points")
        await store.send(.received(event("e1", .userMessage("one"))))
        await store.send(.received(event("e2", .turnStarted)))
        await store.send(.received(event("e3", .assistantMessage("done"))))
        await store.send(.received(event("e4", .compacted)))
        await store.send(.received(event("e5", .turnEnded(aborted: false))))

        await store.send(.rewindTapped("e3"))
        XCTAssertNil(store.state.rewindDialog)
        await store.send(.rewindTapped("e4"))
        XCTAssertNotNil(store.state.rewindDialog)
    }

    // MARK: /compact

    func testCompactWithWhatToKeepInMindStartsACompactionAndEmptiesTheBox() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        try await daemon.client.send(session.id, "one", .enqueue)
        let store = store(daemon, session)

        await store.send(\.binding.draft, "/")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        await store.send(\.binding.draft, "/compact keep the API notes")
        await store.send(.sendTapped(.enqueue))
        await store.receive(\.compactStarted, timeout: .seconds(1))
        XCTAssertEqual(store.state.draft, "")
        XCTAssertFalse(store.state.compactStarting)
        let asked = try XCTUnwrap(index(of: .compacting(focus: "keep the API notes"), in: daemon, session))
        let done = try XCTUnwrap(index(of: .compacted, in: daemon, session))
        XCTAssertLessThan(asked, done)
        XCTAssertFalse(
            daemon.history(session.id).contains { $0.kind == .userMessage("/compact keep the API notes") },
            "the command isn't sent as a message")
    }

    func testPickingCompactFromTheListWaitsForWhatToKeepInMindOnlyWhenTheAgentTakesIt() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        try await daemon.client.send(session.id, "one", .enqueue)
        let store = store(daemon, session)

        await store.send(\.binding.draft, "/comp")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        await store.send(.slashPicked("compact"))
        XCTAssertEqual(store.state.draft, "/compact ")
        XCTAssertEqual(compactions(daemon, session), [])

        daemon.setChatCommands(ChatCommands(compact: true, compactTakesFocus: false))
        let plain = try await started(daemon)
        try await daemon.client.send(plain.id, "one", .enqueue)
        let other = self.store(daemon, plain)
        await other.send(\.binding.draft, "/comp")
        await other.receive(\.slashLoaded, timeout: .seconds(1))
        await other.send(.slashPicked("compact"))
        await other.receive(\.compactStarted, timeout: .seconds(1))
        XCTAssertEqual(other.state.draft, "")
        XCTAssertNotNil(index(of: .compacted, in: daemon, plain))
    }

    func testCompactingWhileATurnRunsAsksFirstAndStoppingKeepsWhatIsQueued() async throws {
        let daemon = InMemoryDaemon { text in
            text == "work" ? [.run(command: "make", output: "built")] : [.say("You said: \(text)")]
        }
        let session = try await started(daemon)
        try await daemon.client.send(session.id, "one", .enqueue)
        try await daemon.client.send(session.id, "work", .enqueue)
        try await daemon.client.send(session.id, "later", .enqueue)
        let store = store(daemon, session)
        let watching = await store.send(.task)
        await store.receive(\.receivedAll, timeout: .seconds(1))
        XCTAssertTrue(store.state.isWorking)

        await store.send(.compactAsked(focus: ""))
        XCTAssertNotNil(store.state.compactDialog)
        await store.send(.compactDialog(.dismiss))
        XCTAssertNil(store.state.compactDialog)
        XCTAssertEqual(compactions(daemon, session), [], "cancelling leaves the turn as it was")
        XCTAssertNil(index(of: .turnEnded(aborted: true), in: daemon, session))

        await store.send(.compactAsked(focus: ""))
        await store.send(.compactDialog(.presented(.stopAndCompact(focus: ""))))
        await store.receive(\.compactStarted, timeout: .seconds(1))
        let stopped = try XCTUnwrap(index(of: .turnEnded(aborted: true), in: daemon, session))
        let asked = try XCTUnwrap(index(of: .compacting(focus: ""), in: daemon, session))
        let done = try XCTUnwrap(index(of: .compacted, in: daemon, session))
        let later = try XCTUnwrap(index(of: .userMessage("later"), in: daemon, session))
        XCTAssertLessThan(stopped, asked)
        XCTAssertLessThan(asked, done)
        XCTAssertLessThan(done, later, "what was queued runs after the compaction")
        await watching.cancel()
    }

    func testACompactionThatCantStartSaysWhyAndPutsWhatToKeepInMindBack() async throws {
        let daemon = InMemoryDaemon()
        // Nothing has been said yet, so there is nothing to sum up.
        let store = store(daemon, try await started(daemon))

        await store.send(.compactAsked(focus: "the API notes"))
        await store.receive(\.compactFailed, timeout: .seconds(1))
        XCTAssertNotNil(store.state.alert)
        XCTAssertFalse(store.state.compactStarting)
        XCTAssertEqual(SlashChoice.command(in: store.state.draft)?.command, .compact)
        XCTAssertEqual(SlashChoice.command(in: store.state.draft)?.text, "the API notes")
    }

    func testATurnIsCompactingFromWhenItIsAskedForUntilItEnds() async {
        let stopped = LockIsolated(0)
        let store = fed("compacting") { $0.daemon.abort = { _ in stopped.withValue { $0 += 1 } } }
        await store.send(.received(event("e1", .userMessage("one"))))
        await store.send(.received(event("e2", .turnStarted)))
        XCTAssertFalse(store.state.isCompacting)
        await store.send(.received(event("e3", .turnEnded(aborted: false))))

        await store.send(.received(event("e4", .compacting(focus: ""))))
        await store.send(.received(event("e5", .turnStarted)))
        XCTAssertTrue(store.state.isCompacting)
        XCTAssertTrue(store.state.isWorking)
        // It holds no message of the person's to stop and edit.
        await store.send(.stopAndEditTapped)
        await store.finish()
        XCTAssertEqual(stopped.value, 0)
        XCTAssertEqual(store.state.draft, "")

        await store.send(.received(event("e6", .compacted)))
        await store.send(.received(event("e7", .turnEnded(aborted: false))))
        XCTAssertFalse(store.state.isCompacting)
        XCTAssertEqual(store.state.items.last, .compacted(id: "e6"))

        await store.send(.received(event("e8", .userMessage("two"))))
        await store.send(.received(event("e9", .turnStarted)))
        XCTAssertFalse(store.state.isCompacting, "the next message's turn is its own")
    }

    func testAFailedCompactionOffersNoTryAgainAndAFailedMessageAfterItDoes() async {
        let store = fed("compaction-failed")
        await store.send(.received(event("e1", .userMessage("one"))))
        await store.send(.received(event("e2", .turnStarted)))
        await store.send(.received(event("e3", .turnEnded(aborted: false))))
        await store.send(.received(event("e4", .compacting(focus: ""))))
        await store.send(.received(event("e5", .turnStarted)))
        await store.send(.received(event("e6", .failed("The agent couldn't sum it up"))))
        await store.send(.received(event("e7", .turnEnded(aborted: false))))
        XCTAssertTrue(store.state.items.contains(.failure(id: "e6", message: "The agent couldn't sum it up")))
        XCTAssertNil(store.state.retryableFailure)

        await store.send(.received(event("e8", .userMessage("two"))))
        await store.send(.received(event("e9", .turnStarted)))
        await store.send(.received(event("e10", .failed("The model went away"))))
        await store.send(.received(event("e11", .turnEnded(aborted: false))))
        XCTAssertEqual(store.state.retryableFailure, "e10")
    }

    func testACompactionEndsTheChanceToUndoARewind() {
        let rewound = [
            event("e1", .userMessage("one")), event("e2", .turnStarted), event("e3", .turnEnded(aborted: false)),
            event("e4", .rewound(text: "two", files: false)),
        ]
        XCTAssertEqual(ChatItem.items(rewound).last, .rewound(id: "e4", files: false, undoable: true))
        let compacting = rewound + [event("e5", .compacting(focus: ""))]
        XCTAssertEqual(ChatItem.items(compacting).last, .rewound(id: "e4", files: false, undoable: false))
    }

    // MARK: /new-here, /fork and /side

    func testNewHereStartsAnEmptyChatUnderTheSessionInTheSameCopyAndOpensIt() async throws {
        let daemon = InMemoryDaemon { _ in [.write(path: "a.txt", contents: "one")] }
        let session = try await started(daemon)
        try await daemon.client.setApprovalMode(session.id, .allowEverything)
        try await daemon.client.send(session.id, "one", .enqueue)
        let store = store(daemon, session)

        await store.send(\.binding.draft, "/new-here")
        await store.send(.sendTapped(.enqueue))
        await store.receive(\.delegate.openSession, timeout: .seconds(1))
        let child = try XCTUnwrap(daemon.sessions.last)
        XCTAssertEqual(child.parentID, session.id)
        XCTAssertEqual(child.workingDirectory, session.workingDirectory)
        XCTAssertEqual(daemon.history(child.id), [])
        XCTAssertEqual(store.state.draft, "")
        XCTAssertFalse(store.state.forking)
        XCTAssertFalse(
            daemon.history(session.id).contains { $0.kind == .userMessage("/new-here") },
            "the command isn't sent as a message")
    }

    func testNewHereWithACopyStartsFromTheCodeAsItIsAndLeavesTheSessionsAlone() async throws {
        let daemon = InMemoryDaemon { text in [.write(path: "a.txt", contents: text)] }
        let project = try await daemon.client.createProject(
            "App", [Repository(name: "app", path: "/code/app")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, name: "first", approvalMode: .allowEverything, projectID: project.id))
        try await daemon.client.send(session.id, "one", .enqueue)
        let store = store(daemon, session)

        await store.send(\.binding.draft, "/new-here copy")
        await store.send(.sendTapped(.enqueue))
        await store.receive(\.delegate.openSession, timeout: .seconds(1))
        let child = try XCTUnwrap(daemon.sessions.last)
        XCTAssertEqual(daemon.history(child.id), [])
        XCTAssertEqual(daemon.files(of: child.id), ["a.txt": "one"])

        try await daemon.client.send(child.id, "two", .enqueue)
        XCTAssertEqual(daemon.files(of: child.id), ["a.txt": "two"])
        XCTAssertEqual(daemon.files(of: session.id), ["a.txt": "one"])
    }

    func testForkForksTheChatSoFarAndWithChatSharesTheCopy() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        try await daemon.client.send(session.id, "one", .enqueue)
        let store = store(daemon, session)

        await store.send(\.binding.draft, "/fork chat")
        await store.send(.sendTapped(.enqueue))
        await store.receive(\.delegate.openSession, timeout: .seconds(1))
        let fork = try XCTUnwrap(daemon.sessions.last)
        XCTAssertEqual(fork.parentID, session.id)
        XCTAssertEqual(fork.workingDirectory, session.workingDirectory)
        XCTAssertEqual(daemon.history(fork.id), daemon.history(session.id))
        XCTAssertFalse(fork.readOnly)
        XCTAssertEqual(store.state.draft, "")
    }

    func testSideOpensAReadOnlyChatBesideWithTheChatSoFarAndSendsItTheQuestion() async throws {
        let daemon = InMemoryDaemon { text in
            text == "one" ? [.say("done")] : [.write(path: "a.txt", contents: "changed"), .say("answered")]
        }
        let session = try await started(daemon)
        try await daemon.client.setApprovalMode(session.id, .allowEverything)
        try await daemon.client.send(session.id, "one", .enqueue)
        let before = daemon.history(session.id)
        let store = store(daemon, session)

        await store.send(\.binding.draft, "/")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        await store.send(\.binding.draft, "/side why is it slow?")
        await store.send(.sendTapped(.enqueue))
        var opened: Session?
        await store.receive(
            { action in
                guard case .delegate(.openSide(let side)) = action else { return false }
                opened = side
                return true
            }, timeout: .seconds(1))
        let side = try XCTUnwrap(opened)
        XCTAssertEqual(side.parentID, session.id)
        XCTAssertTrue(side.readOnly)
        XCTAssertEqual(side.workingDirectory, session.workingDirectory)
        XCTAssertEqual(store.state.draft, "")

        let history = daemon.history(side.id)
        XCTAssertEqual(Array(history.prefix(before.count)), before, "it starts with the chat so far")
        XCTAssertTrue(history.contains { $0.kind == .userMessage("why is it slow?") })
        XCTAssertTrue(history.contains { $0.kind == .assistantMessage("answered") })
        XCTAssertEqual(daemon.files(of: session.id), [:], "its write was refused, though the session allows all")
        XCTAssertEqual(daemon.history(session.id), before, "the session itself is as it was")
    }

    func testSideIsOfferedOnlyWhereTheHostHoldsItToReadingAndElseGoesToTheAgentAsTyped() async throws {
        let daemon = InMemoryDaemon()
        daemon.setChatCommands(ChatCommands(compact: true, compactTakesFocus: true))
        let session = try await started(daemon)
        let store = store(daemon, session)

        await store.send(\.binding.draft, "/")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        XCTAssertFalse(store.state.slashChoices.contains { $0.builtIn == .side })

        await store.send(\.binding.draft, "/side why?")
        await store.send(.sendTapped(.enqueue))
        var sent: String?
        await store.receive({ action in
            guard case .delegate(.send(let outgoing)) = action else { return false }
            sent = outgoing.text
            return true
        })
        XCTAssertEqual(sent, "/side why?")
        XCTAssertEqual(daemon.sessions.count, 1, "no chat was made")
    }

    func testASideChatThatCantBeMadeSaysSoAndKeepsTheQuestionInTheBox() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        let store = store(daemon, session)
        await store.send(\.binding.draft, "/")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        // The host stops offering side chats after the list was read.
        daemon.setChatCommands(ChatCommands(compact: true, compactTakesFocus: true))

        await store.send(\.binding.draft, "/side why is it slow?")
        await store.send(.sendTapped(.enqueue))
        await store.receive(\.failed, timeout: .seconds(1))
        XCTAssertNotNil(store.state.alert)
        XCTAssertFalse(store.state.forking)
        XCTAssertEqual(store.state.draft, "/side why is it slow?")
        XCTAssertEqual(daemon.sessions.count, 1)
    }

    func testSideSentBeforeTheHostHasSaidWhatItTakesOpensTheSideChatOnceItHas() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        try await daemon.client.send(session.id, "one", .enqueue)
        let before = daemon.history(session.id)
        let store = store(daemon, session)
        XCTAssertNil(store.state.chatCommands, "the host hasn't been asked yet")

        // Pasted whole, so the / list never opened and asked.
        await store.send(\.binding.draft, "/side why is it slow?")
        await store.send(.sendTapped(.enqueue))
        var opened: Session?
        await store.receive(
            { action in
                guard case .delegate(.openSide(let side)) = action else { return false }
                opened = side
                return true
            }, timeout: .seconds(1))
        let side = try XCTUnwrap(opened)
        XCTAssertTrue(side.readOnly)
        XCTAssertFalse(store.state.holdingSend)
        XCTAssertEqual(store.state.draft, "")
        XCTAssertTrue(daemon.history(side.id).contains { $0.kind == .userMessage("why is it slow?") })
        XCTAssertEqual(daemon.history(session.id), before, "nothing went to the session itself")
    }

    func testACommandHeldForTheHostsAnswerGoesToTheAgentAsTypedWhenTheHostDoesntTakeIt() async throws {
        let daemon = InMemoryDaemon()
        daemon.setChatCommands(ChatCommands())
        let session = try await started(daemon)
        let store = store(daemon, session)

        await store.send(\.binding.draft, "/compact the notes")
        await store.send(.sendTapped(.enqueue))
        var sent: String?
        await store.receive(
            { action in
                guard case .delegate(.send(let outgoing)) = action else { return false }
                sent = outgoing.text
                return true
            }, timeout: .seconds(1))
        XCTAssertEqual(sent, "/compact the notes")
    }

    func testSideChatIsOfferedOnceTheChatOpensAndAHostThatCantSayLaterKeepsWhatItSaid() async throws {
        let daemon = skilled("release")
        let session = try await started(daemon)
        let store = store(daemon, session)

        let watching = await store.send(.task)
        await store.receive(\.commandsLoaded, timeout: .seconds(1))
        XCTAssertEqual(store.state.chatCommands?.side, true, "with nothing typed")
        XCTAssertEqual(store.state.chatCommands?.compact, true)

        // The host can't be asked the next time the / list looks.
        daemon.setChatCommands(nil)
        await store.send(\.binding.draft, "/")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        XCTAssertEqual(store.state.chatCommands?.side, true)
        XCTAssertTrue(store.state.slashChoices.contains { $0.builtIn == .side })
        XCTAssertTrue(store.state.slashChoices.contains { $0.builtIn == .compact })
        await watching.cancel()
    }

    func testAChatOpenedAgainKnowsItsCommandsAtOnce() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        let store = store(daemon, session)
        let watching = await store.send(.task)
        await store.receive(\.commandsLoaded, timeout: .seconds(1))
        await watching.cancel()

        let again = SessionFeature.State(session: session)
        XCTAssertEqual(again.chatCommands, store.state.chatCommands)
        XCTAssertNotNil(again.chatCommands)
    }

    func testASideChatWhoseQuestionCouldntBeSentOpensAndSaysTheQuestionWaitsInItsBox() async throws {
        let daemon = InMemoryDaemon()
        let session = try await started(daemon)
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.send = { _, _, _ in throw DaemonError.disconnected }
            $0.continuousClock = ImmediateClock()
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_000))
        }
        store.exhaustivity = .off
        await store.send(\.binding.draft, "/")
        await store.receive(\.slashLoaded, timeout: .seconds(1))

        await store.send(\.binding.draft, "/side why is it slow?")
        await store.send(.sendTapped(.enqueue))
        var opened: Session?
        await store.receive(
            { action in
                guard case .delegate(.openSide(let side)) = action else { return false }
                opened = side
                return true
            }, timeout: .seconds(1))
        let side = try XCTUnwrap(opened)
        XCTAssertNotNil(store.state.alert, "it says the question didn't reach the side chat")
        XCTAssertFalse(store.state.forking)
        XCTAssertEqual(store.state.draft, "")
        XCTAssertFalse(daemon.history(side.id).contains { $0.kind == .userMessage("why is it slow?") })
        XCTAssertTrue(SessionFeature.State(session: side).draft.contains("why is it slow?"))
    }

    // MARK: What the agent asks for, answered from the strip

    func testAllowingOrDecliningASessionTheAgentAsksForFromTheStripAnswersTheRequest() async {
        let store = fed("strip-request")
        let request = AgentRequest(
            id: "r1", sessionID: "strip-request", kind: .startSession(name: "Tests"), text: "Write the tests.",
            created: Date(timeIntervalSince1970: 0))
        await store.send(\.binding.request, request)

        func answered(_ expected: RequestAnswer) -> (SessionFeature.Action) -> Bool {
            { action in
                guard case .delegate(.answerRequest("r1", let answer)) = action else { return false }
                return answer == expected
            }
        }
        await store.send(.requestAllowed)
        await store.receive(answered(.approve))
        await store.send(.requestDeclined)
        await store.receive(answered(.dismiss))
    }

    func testAQuestionIsntAllowedOrDeclinedFromTheStrip() async {
        let store = fed("strip-question")
        let request = AgentRequest(
            id: "r1", sessionID: "strip-question", kind: .question, text: "Which one?",
            created: Date(timeIntervalSince1970: 0))
        await store.send(\.binding.request, request)
        await store.send(.requestAllowed)
        await store.send(.requestDeclined)
        await store.finish()
    }

    // MARK: The /rewind picker's search

    func testThePickerFindsAMessageWhateverTheCaseOfItOrOfWhatIsTyped() {
        let events = [
            event("e1", .userMessage("Fix the Parser")), event("e2", .userMessage("add tests\nfor the PARSER")),
            event("e3", .userMessage("ship it")), event("e4", .compacted),
        ]
        let points = RewindPoint.points(in: events)
        XCTAssertEqual(RewindPoint.matching("parser", in: points).map(\.id), ["e2", "e1"])
        XCTAssertEqual(RewindPoint.matching("PARSER", in: points).map(\.id), ["e2", "e1"])
        XCTAssertEqual(RewindPoint.matching("Fix", in: points).map(\.id), ["e1"])
        XCTAssertEqual(RewindPoint.matching("COMPACT", in: points).map(\.id), ["e4"])
        // Its letters in order alone don't find a message.
        XCTAssertEqual(RewindPoint.matching("fxp", in: points), [])
        XCTAssertEqual(RewindPoint.matching("", in: points).map(\.id), ["e4", "e3", "e2", "e1"])
    }
}
