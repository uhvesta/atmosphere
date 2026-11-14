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

    func testAFileIsReadWithTheLinesItThenShows() async throws {
        let (daemon, session) = try await chat()
        let store = store(daemon, session)
        let carried = LockIsolated<[String]?>(nil)
        await store.send(.refresh)
        await store.receive { action in
            guard case .read(_, let lines, _, _) = action else { return false }
            carried.setValue(lines)
            return true
        }
        XCTAssertEqual(carried.value, ["# Plan", "", "Ship the reader.", "Then the chat."])
        XCTAssertEqual(store.state.lines, carried.value)
    }

    func testACommentAddedWhileTheOthersAreSentStaysADraft() async throws {
        let (daemon, session) = try await chat()
        let (held, release) = AsyncStream.makeStream(of: Void.self)
        var client = daemon.client
        let send = client.send
        client.send = { session, text, mode in
            for await _ in held { break }
            try await send(session, text, mode)
        }
        let store = TestStore(initialState: DocsFeature.State(sessionID: session.id)) {
            DocsFeature()
        } withDependencies: {
            $0.daemon = client
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.read)
        await store.send(.lineTapped(1, extending: false))
        await store.send(.binding(.set(\.comment, "Rename it.")))
        await store.send(.commentSaved)

        await store.send(.sendTapped)
        await store.send(.lineTapped(3, extending: false))
        await store.send(.binding(.set(\.comment, "Say which reader.")))
        await store.send(.commentSaved)
        XCTAssertEqual(store.state.drafts.count, 2)

        release.yield()
        await store.receive(\.sent)
        XCTAssertEqual(store.state.drafts.map(\.text), ["Say which reader."])
        XCTAssertFalse(store.state.sending)
    }

    func testAClickedBlocksQuoteIsCutShortButSelectedTextIsKeptWhole() async throws {
        let (daemon, session) = try await chat()
        let store = store(daemon, session)
        await store.send(.refresh)
        await store.receive(\.read)
        let long = (1...40).map { "Line \($0) of a long block." }.joined(separator: "\n")

        await store.send(.blockTapped(startLine: 1, endLine: 40, quote: long))
        let quote = try XCTUnwrap(store.state.picked?.quote)
        XCTAssertLessThan(quote.count, long.count)
        XCTAssertTrue(quote.hasPrefix("Line 1 of a long block.\nLine 2 of a long block."))
        XCTAssertFalse(quote.contains("Line 40"))

        await store.send(.textSelected(startLine: 1, endLine: 40, quote: long))
        XCTAssertEqual(store.state.picked?.quote, long)

        await store.send(.blockTapped(startLine: 3, endLine: 4, quote: "Ship the reader.\nThen the chat."))
        XCTAssertEqual(store.state.picked?.quote, "Ship the reader.\nThen the chat.")
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

@MainActor
final class DocsFocusAndVersionsTests: XCTestCase {
    private let first = "# Plan\n\nShip the reader.\n\nThen the chat.\n"
    private let second = "# Plan\n\nShip the reader first.\n\nThen the chat.\n"

    /// A chat whose agent writes the first plan, then the second.
    private func chat() async throws -> (InMemoryDaemon, Session) {
        let (first, second) = (first, second)
        let daemon = InMemoryDaemon { text in
            switch text {
            case "Write": [.write(path: "plan.md", contents: first), .write(path: "notes.txt", contents: "a\n")]
            case "Change": [.write(path: "plan.md", contents: second)]
            default: [.say("Done.")]
            }
        }
        let session = try await daemon.client.start(StartRequest(backend: .scripted, approvalMode: .allowEverything))
        try await daemon.client.send(session.id, "Write", .enqueue)
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

    func testFocusShowsTheFileOnItsOwnAndBack() async throws {
        let (daemon, session) = try await chat()
        let store = store(daemon, session)
        await store.send(.focusToggled) {
            $0.focused = true
        }
        await store.send(.focusToggled) {
            $0.focused = false
        }
    }

    func testAnEditReloadsWithWhatChangedMarkedUntilAnotherFileIsPicked() async throws {
        let (daemon, session) = try await chat()
        let store = store(daemon, session)
        await store.send(.refresh)
        await store.receive(\.read)
        XCTAssertEqual(store.state.changed, [], "Nothing is marked the first time a file is read")

        try await daemon.client.send(session.id, "Change", .enqueue)
        await store.send(.refresh)
        await store.receive(\.read)
        XCTAssertEqual(store.state.changed, [3])
        XCTAssertEqual(store.state.lines?[2], "Ship the reader first.")

        // Read again unchanged, it keeps its marks.
        await store.send(.refresh)
        await store.receive(\.read)
        XCTAssertEqual(store.state.changed, [3])

        await store.send(.fileSelected("notes.txt")) {
            $0.changed = []
        }
    }

    func testAVersionIsShownComparedWithNowAndRestoredAsANewChange() async throws {
        let (daemon, session) = try await chat()
        try await daemon.client.send(session.id, "Change", .enqueue)
        let store = store(daemon, session)
        await store.send(.refresh)
        await store.receive(\.read)

        await store.send(.versionsToggled)
        await store.receive(\.versionsListed)
        XCTAssertEqual(store.state.versions?.list?.map(\.turn), [1, 2])

        await store.send(.versionPicked(1))
        await store.receive(\.versionRead)
        XCTAssertEqual(store.state.versions?.lines?[2], "Ship the reader.")

        await store.send(.compareToggled)
        await store.receive(\.compared)
        let comparison = try XCTUnwrap(store.state.versions?.comparison)
        XCTAssertEqual(comparison.filter { $0.change == .removed }.map(\.block.text), ["Ship the reader."])
        XCTAssertEqual(comparison.filter { $0.change == .added }.map(\.block.text), ["Ship the reader first."])
        XCTAssertEqual(comparison.filter { $0.change == .kept }.count, 2)

        await store.send(.restoreTapped)
        XCTAssertNotNil(store.state.alert, "Restoring asks first")
        await store.send(.alert(.presented(.restore(path: "plan.md", turn: 1))))
        await store.receive(\.restored)
        XCTAssertNil(store.state.versions?.selected)
        XCTAssertEqual(store.state.lines?[2], "Ship the reader.")
        XCTAssertEqual(store.state.changed, [3])
        let now = try await daemon.client.readFile(session.id, "plan.md")
        XCTAssertEqual(now.text, first)

        // The restore is a change of its own, kept by the next turn.
        try await daemon.client.send(session.id, "Carry on", .enqueue)
        await store.send(.refresh)
        await store.receive(\.versionsListed)
        XCTAssertEqual(store.state.versions?.list?.map(\.turn), [1, 2, 3])
    }

    func testADismissedRestoreChangesNothing() async throws {
        let (daemon, session) = try await chat()
        try await daemon.client.send(session.id, "Change", .enqueue)
        let store = store(daemon, session)
        await store.send(.refresh)
        await store.receive(\.read)
        await store.send(.versionsToggled)
        await store.receive(\.versionsListed)
        await store.send(.versionPicked(1))
        await store.receive(\.versionRead)
        await store.send(.restoreTapped)
        await store.send(.alert(.dismiss))
        XCTAssertEqual(store.state.versions?.selected, 1)
        let now = try await daemon.client.readFile(session.id, "plan.md")
        XCTAssertEqual(now.text, second)
    }

    func testAVersionThatCantBeReadGoesBackToTheFileNowAndCanBePickedAgain() async throws {
        let (daemon, session) = try await chat()
        try await daemon.client.send(session.id, "Change", .enqueue)
        let failing = LockIsolated(true)
        var client = daemon.client
        let read = client.readFileVersion
        client.readFileVersion = { session, path, turn in
            if failing.value { throw DaemonError.unexpected("the host is offline") }
            return try await read(session, path, turn)
        }
        let store = TestStore(initialState: DocsFeature.State(sessionID: session.id)) {
            DocsFeature()
        } withDependencies: {
            $0.daemon = client
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.read)
        await store.send(.versionsToggled)
        await store.receive(\.versionsListed)

        await store.send(.versionPicked(1))
        await store.receive(\.versionsFailed)
        XCTAssertNil(store.state.versions?.selected)
        XCTAssertNotNil(store.state.versions?.list)
        XCTAssertNotNil(store.state.alert)

        failing.setValue(false)
        await store.send(.alert(.dismiss))
        await store.send(.versionPicked(1))
        await store.receive(\.versionRead)
        XCTAssertEqual(store.state.versions?.selected, 1)
        XCTAssertNotNil(store.state.versions?.lines)
    }

    func testARestoreThatFailsStopsRestoringAndKeepsTheVersionShown() async throws {
        let (daemon, session) = try await chat()
        try await daemon.client.send(session.id, "Change", .enqueue)
        var client = daemon.client
        client.restoreFileVersion = { _, _, _ in throw DaemonError.unexpected("refused") }
        let store = TestStore(initialState: DocsFeature.State(sessionID: session.id)) {
            DocsFeature()
        } withDependencies: {
            $0.daemon = client
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.read)
        await store.send(.versionsToggled)
        await store.receive(\.versionsListed)
        await store.send(.versionPicked(1))
        await store.receive(\.versionRead)
        await store.send(.restoreTapped)
        await store.send(.alert(.presented(.restore(path: "plan.md", turn: 1))))
        await store.receive(\.versionsFailed)
        XCTAssertEqual(store.state.versions?.restoring, false)
        XCTAssertEqual(store.state.versions?.selected, 1)
        XCTAssertNotNil(store.state.alert)
    }

    func testAReadBegunBeforeARestoreDoesntShowOverIt() async throws {
        let (daemon, session) = try await chat()
        try await daemon.client.send(session.id, "Change", .enqueue)
        // The next read holds the file as it is now until let go.
        let holding = LockIsolated(false)
        let (started, readStarted) = AsyncStream<Void>.makeStream()
        let (release, letGo) = AsyncStream<Void>.makeStream()
        var client = daemon.client
        let readFile = client.readFile
        client.readFile = { session, path in
            let contents = try await readFile(session, path)
            if holding.value {
                readStarted.yield()
                for await _ in release { break }
            }
            return contents
        }
        let store = TestStore(initialState: DocsFeature.State(sessionID: session.id)) {
            DocsFeature()
        } withDependencies: {
            $0.daemon = client
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.read)
        await store.send(.versionsToggled)
        await store.receive(\.versionsListed)
        await store.send(.versionPicked(1))
        await store.receive(\.versionRead)

        holding.setValue(true)
        await store.send(.refresh)
        for await _ in started { break }
        holding.setValue(false)

        await store.send(.restoreTapped)
        await store.send(.alert(.presented(.restore(path: "plan.md", turn: 1))))
        await store.receive(\.restored)
        XCTAssertEqual(store.state.lines?[2], "Ship the reader.")

        letGo.finish()
        await store.receive(\.read)
        XCTAssertEqual(store.state.lines?[2], "Ship the reader.", "The late read is of the file before the restore")
    }

    func testVersionsThatCantBeReadShowAnAlert() async throws {
        let daemon = InMemoryDaemon()
        var state = DocsFeature.State(sessionID: "gone")
        state.selected = "plan.md"
        let store = TestStore(initialState: state) {
            DocsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        await store.send(.versionsToggled)
        await store.receive(\.versionsFailed) {
            $0.versions = nil
        }
        XCTAssertNotNil(store.state.alert)
    }
}

extension SessionEvent {
    fileprivate var isUserMessage: Bool {
        if case .userMessage = kind { true } else { false }
    }
}
