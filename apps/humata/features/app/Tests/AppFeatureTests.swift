import ComposableArchitecture
import HumataAppFeature
import HumataBoard
import HumataDaemonClient
import HumataHosts
import HumataProjects
import HumataReview
import HumataSearch
import HumataSession
import XCTest

@MainActor
final class AppFeatureTests: XCTestCase {
    func testPickingASessionOpensItsChat() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        var state = AppFeature.State()
        state.sidebar.sessions = [session]
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.selected, .session(session.id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.session, session)
    }

    func testADocCommentAndTheTabSurviveLookingAtTheInbox() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        var state = AppFeature.State()
        state.sidebar.sessions = [session]
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.selected, .session(session.id))
        await store.receive(\.sidebar.delegate.openSession)
        await store.send(\.session.tabPicked, .files)
        await store.send(\.session.docs.fileSelected, "plan.md")
        await store.send(\.session.docs.blockTapped, (startLine: 1, endLine: 1, quote: "Ship it."))
        await store.send(\.session.docs.binding.comment, "Not yet.")
        await store.send(\.session.docs.commentSaved)
        XCTAssertEqual(store.state.session?.docs.drafts.count, 1)

        await store.send(\.sidebar.selected, .inbox)
        await store.send(\.sidebar.selected, .session(session.id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.tab, .files)
        XCTAssertEqual(store.state.session?.docs.drafts.map(\.text), ["Not yet."])
    }

    func testAReviewCommentAndAnEditedMessageSurviveLookingAtTheInbox() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        let file = FileRef(repository: "work", path: "main.swift")
        var state = AppFeature.State()
        state.sidebar.sessions = [session]
        var open = SessionFeature.State(session: session)
        open.tab = .review
        open.review.selected = file
        open.review.commenting = ReviewFeature.Commenting(file: file, startLine: 3, endLine: 4)
        open.review.comment = "Why this?"
        open.editing = "e1"
        open.edited = "Try again, smaller"
        state.session = open
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.selected, .inbox)
        await store.receive(\.sidebar.delegate.openInbox)
        XCTAssertNil(store.state.session)
        await store.send(\.sidebar.selected, .session(session.id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.tab, .review)
        XCTAssertEqual(store.state.session?.review.commenting?.file, file)
        XCTAssertEqual(store.state.session?.review.comment, "Why this?")
        XCTAssertEqual(store.state.session?.editing, "e1")
        XCTAssertEqual(store.state.session?.edited, "Try again, smaller")
    }

    func testAnOpenSessionShowsItsMoveRequestUntilItIsAnswered() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, projectID: project.id, itemID: task.id))
        let move = daemon.addRequest(session.id, .move(item: task.id, status: .inReview), "Login works.")
        let store = TestStore(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.notifications.post = { _ in }
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loaded)
        await store.send(\.sidebar.selected, .session(session.id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.request?.id, move.id)

        await store.send(\.session.moveApproved)
        await store.receive(\.sidebar.requestAnswered)
        await store.receive(\.sidebar.loaded)
        XCTAssertNil(store.state.session?.request)
        let status = try await daemon.client.items(project.id).first?.status
        XCTAssertEqual(status, .inReview)
    }

    func testPickingTheOpenSessionAgainKeepsItsChat() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = AppFeature.State()
        state.sidebar.sessions = [session]
        var open = SessionFeature.State(session: session)
        open.draft = "half written"
        state.session = open
        let store = TestStore(initialState: state) { AppFeature() }
        store.exhaustivity = .off

        await store.send(\.sidebar.selected, .session(session.id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.draft, "half written")
    }

    func testAForkOpensAndIsPickedInTheSidebar() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", name: "first"))
        try await daemon.client.send(session.id, "hello", .enqueue)
        var state = AppFeature.State()
        state.sidebar.sessions = [session]
        state.session = SessionFeature.State(session: session)
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.session.forkTapped, (before: nil, chatOnly: false))
        await store.receive(\.session.delegate.openSession)
        await store.receive(\.sidebar.loaded)
        let fork = try XCTUnwrap(store.state.session?.session)
        XCTAssertNotEqual(fork.id, session.id)
        XCTAssertEqual(fork.parentID, session.id)
        XCTAssertEqual(store.state.sidebar.selection, .session(fork.id))
        XCTAssertEqual(Set(store.state.sidebar.sessions.ids), [session.id, fork.id])
    }

    /// A store with `first` open and a side chat of it beside, asked
    /// `question`.
    private func withASideChat(
        _ daemon: InMemoryDaemon, of first: Session, others: [Session] = [], question: String = ""
    ) async throws -> TestStoreOf<AppFeature> {
        var state = AppFeature.State()
        state.sidebar.sessions = IdentifiedArray(uniqueElements: [first] + others)
        state.sidebar.selection = .session(first.id)
        state.session = SessionFeature.State(session: first)
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.notifications.post = { _ in }
        }
        store.exhaustivity = .off
        await store.send(.session(.presented(.childTapped(.side, message: question))))
        await store.receive(\.session.delegate.openSide)
        await store.receive(\.sidebar.loaded)
        return store
    }

    func testASideChatOpensBesideItsSessionTakesItsOwnMessagesAndStaysInTheSidebarWhenClosed() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", name: "first"))
        try await daemon.client.send(session.id, "hello", .enqueue)
        let store = try await withASideChat(daemon, of: session, question: "and why?")

        let side = try XCTUnwrap(store.state.side?.session)
        XCTAssertEqual(store.state.session?.session.id, session.id, "the session stays open")
        XCTAssertEqual(store.state.sidebar.selection, .session(session.id))
        XCTAssertEqual(side.parentID, session.id)
        XCTAssertTrue(side.readOnly)
        XCTAssertEqual(store.state.side?.tabs, [.chat])
        XCTAssertTrue(daemon.history(side.id).contains { $0.kind == .userMessage("and why?") })

        await store.send(\.side.binding.draft, "more")
        await store.send(\.side.sendTapped, .enqueue)
        await store.receive(\.side.sent)
        XCTAssertTrue(daemon.history(side.id).contains { $0.kind == .userMessage("more") })
        XCTAssertFalse(daemon.history(session.id).contains { $0.kind == .userMessage("more") })

        await store.send(.side(.dismiss))
        XCTAssertNil(store.state.side)
        XCTAssertEqual(store.state.session?.session.id, session.id)
        XCTAssertEqual(Set(store.state.sidebar.sessions.ids), [session.id, side.id], "it stays under its session")
    }

    func testOpeningAnotherSessionClosesTheSideChatAndPickingItOpensItAsASession() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", name: "first"))
        let other = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", name: "second"))
        let store = try await withASideChat(daemon, of: session, others: [other])
        let side = try XCTUnwrap(store.state.side?.session)

        await store.send(\.sidebar.selected, .session(other.id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.session.id, other.id)
        XCTAssertNil(store.state.side)

        await store.send(\.sidebar.selected, .session(side.id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.session.id, side.id)
        XCTAssertNil(store.state.side)
    }

    /// A store with `first` open and `side` beside it, whose daemon notes
    /// in `stopped` each session told to stop its turn.
    private func besideEachOther(
        _ first: Session, _ side: Session, stopped: LockIsolated<[Session.ID]>
    ) -> TestStoreOf<AppFeature> {
        var beside = SessionFeature.State(session: side)
        beside.beside = true
        var state = AppFeature.State()
        state.sidebar.sessions = [first, side]
        state.sidebar.selection = .session(first.id)
        state.session = SessionFeature.State(session: first)
        state.side = beside
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.daemon.abort = { session in stopped.withValue { $0.append(session) } }
        }
        store.exhaustivity = .off
        return store
    }

    func testWithTheSideColumnFocusedStopTheTurnStopsTheSideChatsTurnAndNotTheSessions() async throws {
        let first = Session(id: "column-first", name: "first", backend: .scripted, workingDirectory: "/work")
        var side = Session(id: "column-side", name: "side", backend: .scripted, workingDirectory: "/work")
        side.parentID = first.id
        side.readOnly = true
        let stopped = LockIsolated<[Session.ID]>([])
        let store = besideEachOther(first, side, stopped: stopped)

        await store.send(.columnFocused(.side))
        XCTAssertEqual(store.state.side?.takesShortcuts, true)
        XCTAssertEqual(store.state.session?.takesShortcuts, false, "one column answers a shortcut, not both")

        await store.send(.sessionCommand(.abortTapped))
        await store.receive(\.side.abortTapped)
        await store.finish()
        XCTAssertEqual(stopped.value, [side.id], "the session's own turn keeps running")

        // Out of both columns, as in the sidebar, the commands stay with
        // the one last in.
        await store.send(.columnFocused(nil))
        XCTAssertEqual(store.state.commandSession?.session.id, side.id)

        await store.send(.columnFocused(.main))
        XCTAssertEqual(store.state.session?.takesShortcuts, true)
        XCTAssertEqual(store.state.side?.takesShortcuts, false)
        await store.send(.sessionCommand(.abortTapped))
        await store.receive(\.session.abortTapped)
        await store.finish()
        XCTAssertEqual(stopped.value, [side.id, first.id])
    }

    func testClosingTheSideChatGivesCommandsAndShortcutsBackToTheSession() async throws {
        let first = Session(id: "closed-first", name: "first", backend: .scripted, workingDirectory: "/work")
        var side = Session(id: "closed-side", name: "side", backend: .scripted, workingDirectory: "/work")
        side.parentID = first.id
        side.readOnly = true
        let stopped = LockIsolated<[Session.ID]>([])
        let store = besideEachOther(first, side, stopped: stopped)

        await store.send(.columnFocused(.side))
        await store.send(.side(.dismiss))
        XCTAssertNil(store.state.side)
        XCTAssertEqual(store.state.session?.takesShortcuts, true)
        XCTAssertEqual(store.state.commandSession?.session.id, first.id)

        await store.send(.sessionCommand(.abortTapped))
        await store.receive(\.session.abortTapped)
        await store.finish()
        XCTAssertEqual(stopped.value, [first.id])

        // With none open, there's no side column to go into.
        await store.send(.columnFocused(.side))
        XCTAssertEqual(store.state.session?.takesShortcuts, true)
        XCTAssertEqual(store.state.commandSession?.session.id, first.id)
    }

    func testTwoMessagesSentQuicklyReachTheSessionInTheOrderTheyWereSent() async {
        let session = Session(id: "in-order", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = AppFeature.State()
        state.sidebar.sessions = [session]
        state.sidebar.selection = .session(session.id)
        state.session = SessionFeature.State(session: session)
        let sent = LockIsolated<[String]>([])
        // The first send is slow to be answered.
        let (answered, answer) = AsyncStream<Void>.makeStream()
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.daemon.send = { _, text, _ in
                sent.withValue { $0.append(text) }
                if text == "first" {
                    for await _ in answered { break }
                }
            }
        }
        store.exhaustivity = .off

        await store.send(\.session.binding.draft, "first")
        await store.send(\.session.sendTapped, .enqueue)
        await store.receive(\.session.delegate.send)
        await store.send(\.session.binding.draft, "second")
        await store.send(\.session.sendTapped, .enqueue)
        await store.receive(\.session.delegate.send)
        XCTAssertEqual(sent.value, ["first"], "the second waits for the first")

        answer.yield()
        await store.receive(\.sendFinished)
        await store.receive(\.sendFinished)
        XCTAssertEqual(sent.value, ["first", "second"])
        await store.skipInFlightEffects()
    }

    func testAMessageSentAfterOneThatFailedIsStillSent() async {
        let session = Session(id: "after-failed", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = AppFeature.State()
        state.sidebar.sessions = [session]
        state.sidebar.selection = .session(session.id)
        state.session = SessionFeature.State(session: session)
        let sent = LockIsolated<[String]>([])
        let (answered, answer) = AsyncStream<Void>.makeStream()
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.daemon.send = { _, text, _ in
                sent.withValue { $0.append(text) }
                if text == "first" {
                    for await _ in answered { break }
                    throw DaemonError.disconnected
                }
            }
        }
        store.exhaustivity = .off

        await store.send(\.session.binding.draft, "first")
        await store.send(\.session.sendTapped, .enqueue)
        await store.receive(\.session.delegate.send)
        await store.send(\.session.binding.draft, "second")
        await store.send(\.session.sendTapped, .enqueue)
        await store.receive(\.session.delegate.send)

        answer.yield()
        await store.receive(\.sendFailed)
        await store.receive(\.sendFinished)
        XCTAssertEqual(sent.value, ["first", "second"])
        await store.skipInFlightEffects()
    }

    func testAMessageFromASideChatClosedBeforeItsSendFailsGoesBackInItsBox() async throws {
        let first = Session(id: "unsent-beside", name: "first", backend: .scripted, workingDirectory: "/work")
        var side = Session(id: "unsent-side", name: "side", backend: .scripted, workingDirectory: "/work")
        side.parentID = first.id
        side.readOnly = true
        var beside = SessionFeature.State(session: side)
        beside.beside = true
        var state = AppFeature.State()
        state.sidebar.sessions = [first, side]
        state.sidebar.selection = .session(first.id)
        state.session = SessionFeature.State(session: first)
        state.side = beside
        // The send is still under way when the side chat closes.
        let (answered, answer) = AsyncStream<Void>.makeStream()
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.daemon.send = { _, _, _ in
                for await _ in answered { break }
                throw DaemonError.disconnected
            }
        }
        store.exhaustivity = .off

        await store.send(\.side.binding.draft, "and why?")
        await store.send(\.side.sendTapped, .enqueue)
        await store.receive(\.side.delegate.send)
        await store.send(.side(.dismiss))
        XCTAssertNil(store.state.side)

        answer.yield()
        await store.receive(\.sendFailed)
        XCTAssertNotNil(store.state.sendFailure, "it says which chat's message wasn't sent")
        XCTAssertEqual(store.state.session?.draft, "", "the session it was beside is left alone")
        XCTAssertEqual(SessionFeature.State(session: side).draft, "and why?")

        // The alert opens the side chat as a session, the message in its box.
        await store.send(.sendFailure(.presented(.openSession(side.id))))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.session.id, side.id)
        XCTAssertEqual(store.state.session?.draft, "and why?")
    }

    func testASideChatShowsWhatItsAgentAsksAndClosesWhenArchivedElsewhere() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", name: "first"))
        let store = try await withASideChat(daemon, of: session)
        let side = try XCTUnwrap(store.state.side?.session)

        let asked = daemon.addRequest(side.id, .question, "Which of the two?")
        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loaded)
        XCTAssertEqual(store.state.side?.request?.id, asked.id)
        XCTAssertNil(store.state.session?.request, "the session it's beside wasn't asked")

        _ = try await daemon.client.update(side.id, SessionUpdate(archived: true))
        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loaded)
        XCTAssertNil(store.state.side)
        XCTAssertEqual(store.state.session?.session.id, session.id, "the session stays open")
    }

    func testHostsOpenBesideTheSidebarAndCloseForASession() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = AppFeature.State()
        state.sidebar.sessions = [session]
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.selected, .hosts)
        await store.receive(\.sidebar.delegate.openHosts)
        XCTAssertNotNil(store.state.hosts)
        await store.send(\.sidebar.selected, .session(session.id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertNil(store.state.hosts)
        XCTAssertEqual(store.state.session?.session, session)
    }

    func testTheOpenSessionShowsWhenItsHostGoesOffline() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", host: "far"))
        var state = AppFeature.State()
        state.session = SessionFeature.State(session: session)
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        daemon.setOffline("far", true)
        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loaded)
        XCTAssertEqual(store.state.session?.session.hostOffline, true)
    }

    func testASearchHitOpensItsSessionAtTheEventFound() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", name: "first"))
        try await daemon.client.send(session.id, "the migrations fail", .enqueue)
        let message = try XCTUnwrap(daemon.history(session.id).first { $0.kind == .userMessage("the migrations fail") })
        var state = AppFeature.State()
        state.sidebar.sessions = [session]
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.selected, .search)
        await store.receive(\.sidebar.delegate.openSearch)
        XCTAssertNotNil(store.state.search)
        await store.send(\.search.delegate.openSession, (id: session.id, event: message.id))
        XCTAssertNil(store.state.search)
        XCTAssertEqual(store.state.sidebar.selection, .session(session.id))
        XCTAssertEqual(store.state.session?.session.id, session.id)
        XCTAssertEqual(store.state.session?.highlighted, message.id)
    }

    func testASearchHitOnASpecOpensItsBoard() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let item = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        var state = AppFeature.State()
        state.sidebar.projects = [project]
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.selected, .search)
        await store.receive(\.sidebar.delegate.openSearch)
        await store.send(\.search.delegate.openItem, (projectID: project.id, itemID: item.id))
        XCTAssertNil(store.state.search)
        XCTAssertEqual(store.state.board?.project.id, project.id)
        XCTAssertEqual(store.state.board?.opening, item.id)
    }

    func testASessionOpensTheTaskItWorksOn() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let item = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        let onTask = try await daemon.client.start(StartRequest(backend: .scripted, itemID: item.id))
        let onProject = try await daemon.client.start(StartRequest(backend: .scripted, projectID: project.id))
        let sessions = try await daemon.client.sessions(nil)
        XCTAssertEqual(sessions.first { $0.id == onTask.id }?.itemTitle, "login")
        XCTAssertNil(sessions.first { $0.id == onProject.id }?.itemTitle)
        var state = AppFeature.State()
        state.sidebar.projects = [project]
        state.sidebar.sessions = IdentifiedArray(uniqueElements: sessions)
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.selected, .session(onTask.id))
        await store.receive(\.sidebar.delegate.openSession)
        await store.send(\.session.itemTapped)
        await store.receive(\.session.delegate.openItem)
        XCTAssertEqual(store.state.board?.project.id, project.id)
        XCTAssertEqual(store.state.board?.opening, item.id)
    }

    func testALinkOpensASessionTheSidebarHasNotReadYet() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        let url = try XCTUnwrap(URL(string: "humata://session/\(session.id)"))

        await store.send(.opened(url))
        await store.receive(\.sidebar.loaded)
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.sidebar.selection, .session(session.id))
        XCTAssertEqual(store.state.session?.session.id, session.id)
    }

    func testALinkToATaskOpensItsBoardAtTheTask() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let store = TestStore(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        let url = try XCTUnwrap(URL(string: "humata://task/\(project.id)/item-7"))

        await store.send(.opened(url))
        await store.receive(\.sidebar.loaded)
        XCTAssertEqual(store.state.sidebar.selection, .board(project.id))
        XCTAssertEqual(store.state.board?.project.id, project.id)
        XCTAssertEqual(store.state.board?.opening, "item-7")
    }

    func testABoardPutAwayWhileWorkWasStartingStartsItWhenShownAgain() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        _ = try await daemon.client.setSessionDefaults(nil, SessionDefaults(backend: .scripted))
        // Work was starting on the task, and its page was starting it too.
        var board = BoardFeature.State(project: project)
        board.items = [task]
        board.starting = [task.id]
        var page = ItemFeature.State(project: project, item: task, parent: nil, subtasks: [], sessions: [])
        page.startingWork = true
        board.destination = .item(page)
        var state = AppFeature.State()
        state.sidebar.projects = [project]
        state.sidebar.selection = .board(project.id)
        state.board = board
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.selected, .inbox)
        await store.receive(\.sidebar.delegate.openInbox)
        XCTAssertNil(store.state.board)
        await store.send(\.sidebar.selected, .board(project.id))
        await store.receive(\.sidebar.delegate.openBoard)

        await store.send(\.board.startWorkTapped, task.id)
        await store.receive(\.board.startFinished)
        XCTAssertEqual(daemon.sessions.count, 1)
        guard case .item(let shown) = store.state.board?.destination else {
            return XCTFail("the task's page isn't open")
        }
        XCTAssertFalse(shown.startingWork)
    }

    func testSearchAndHostsPutAwayMidwayAreNotStillWaitingWhenShownAgain() async throws {
        var search = SearchFeature.State()
        search.text = "login"
        search.isSearching = true
        var state = AppFeature.State()
        state.search = search
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.calendar = Calendar(identifier: .gregorian)
            $0.daemon = InMemoryDaemon().client
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.selected, .hosts)
        await store.receive(\.sidebar.delegate.openHosts)
        await store.send(\.sidebar.selected, .search)
        await store.receive(\.sidebar.delegate.openSearch)
        XCTAssertEqual(store.state.search?.text, "login")
        XCTAssertEqual(store.state.search?.isSearching, false)
        // The text it was left with is searched once it shows again.
        await store.receive(\.search.finished)
        XCTAssertEqual(store.state.search?.searched, true)
        XCTAssertEqual(store.state.search?.isSearching, false)

        var hosts = HostsFeature.State()
        hosts.linking = true
        hosts.restoring = ["r1"]
        XCTAssertFalse(hosts.settled().linking)
        XCTAssertTrue(hosts.settled().restoring.isEmpty)
    }

    func testSearchAlreadyDoneIsNotSearchedAgainWhenShownAgain() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "the login fails", .enqueue)
        // Searched before the message was sent, so it found nothing.
        var search = SearchFeature.State()
        search.text = "login"
        search.searched = true
        var state = AppFeature.State()
        state.search = search
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.selected, .hosts)
        await store.receive(\.sidebar.delegate.openHosts)
        await store.send(\.sidebar.selected, .search)
        await store.receive(\.sidebar.delegate.openSearch)
        await store.finish()
        XCTAssertEqual(store.state.search?.hits, [])
    }

    func testDeletingTheOpenSessionClosesIt() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        let other = Session(id: "s2", name: "second", backend: .scripted, workingDirectory: "/work")
        var state = AppFeature.State()
        state.sidebar.sessions = [session, other]
        state.sidebar.selection = .session(session.id)
        state.session = SessionFeature.State(session: session)
        let store = TestStore(initialState: state) { AppFeature() }
        store.exhaustivity = .off

        await store.send(\.sidebar.sessionDeleted, other.id)
        XCTAssertEqual(store.state.session?.session.id, session.id)

        await store.send(\.sidebar.sessionDeleted, session.id)
        XCTAssertNil(store.state.session)
        XCTAssertNil(store.state.sidebar.selection)
    }

    /// A store with `open` open, `beside` beside it if given, and both in
    /// the sidebar with `others`, over a daemon that lists none of them.
    private func withOpen(
        _ open: Session, beside: Session? = nil, others: [Session] = []
    ) -> TestStoreOf<AppFeature> {
        var state = AppFeature.State()
        state.sidebar.sessions = IdentifiedArray(uniqueElements: [open] + (beside.map { [$0] } ?? []) + others)
        state.sidebar.selection = .session(open.id)
        state.session = SessionFeature.State(session: open)
        if let beside {
            var side = SessionFeature.State(session: beside)
            side.beside = true
            state.side = side
        }
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
            $0.notifications.post = { _ in }
        }
        store.exhaustivity = .off
        return store
    }

    func testRemovingAHostInSettingsClosesItsOpenSessionAndSideChat() async throws {
        var session = Session(id: "far-1", name: "first", backend: .scripted, workingDirectory: "/work")
        session.host = "far"
        var side = Session(id: "far-2", name: "side", backend: .scripted, workingDirectory: "/work")
        side.host = "far"
        let store = withOpen(session, beside: side)

        await store.send(\.settings.removed, "far")
        XCTAssertNil(store.state.session)
        XCTAssertNil(store.state.side)
        XCTAssertNil(store.state.sidebar.selection)
        // The sidebar reads again, for the sessions that went with the host.
        await store.receive(\.sidebar.loaded)
    }

    func testRemovingAHostInSettingsLeavesAnotherHostsSessionOpen() async throws {
        var session = Session(id: "near-1", name: "first", backend: .scripted, workingDirectory: "/work")
        session.host = "near"
        // This Mac's.
        let side = Session(id: "mac-1", name: "side", backend: .scripted, workingDirectory: "/work")
        let store = withOpen(session, beside: side)

        await store.send(\.settings.removed, "far")
        XCTAssertEqual(store.state.session?.session.id, session.id)
        XCTAssertEqual(store.state.side?.session.id, side.id)
        XCTAssertEqual(store.state.sidebar.selection, .session(session.id))
    }

    func testRemovingAHostInSettingsClosesASideChatOnItAndLeavesTheSessionItIsBeside() async throws {
        let session = Session(id: "mac-2", name: "first", backend: .scripted, workingDirectory: "/work")
        var side = Session(id: "far-3", name: "side", backend: .scripted, workingDirectory: "/work")
        side.host = "far"
        let store = withOpen(session, beside: side)

        await store.send(\.settings.removed, "far")
        XCTAssertEqual(store.state.session?.session.id, session.id)
        XCTAssertNil(store.state.side)
        XCTAssertEqual(store.state.sidebar.selection, .session(session.id))
    }

    func testDeletingAProjectClosesItsBoardAndLeavesItsOpenSessionOnNoProject() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let session = Session(
            id: "s1", name: "first", backend: .scripted, workingDirectory: "/work", projectID: project.id,
            archived: true, itemID: "item-1", itemTitle: "login")
        var open = SessionFeature.State(session: session)
        open.tab = .review
        var state = AppFeature.State()
        state.sidebar.projects = [project]
        state.sidebar.sessions = [session]
        state.session = open
        state.board = BoardFeature.State(project: project)
        let store = TestStore(initialState: state) { AppFeature() }
        store.exhaustivity = .off

        await store.send(\.sidebar.projectDeleted, project.id)
        XCTAssertNil(store.state.board)
        XCTAssertNil(store.state.session?.session.projectID)
        XCTAssertNil(store.state.session?.session.itemID)
        XCTAssertNil(store.state.session?.session.itemTitle)
        XCTAssertEqual(store.state.session?.tab, .chat)
    }

    func testDeletingAProjectDropsTheBoardKeptForIt() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        var board = BoardFeature.State(project: project)
        board.actionFailure = "The card couldn't be moved."
        var state = AppFeature.State()
        state.sidebar.projects = [project]
        state.sidebar.selection = .board(project.id)
        state.board = board
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.selected, .inbox)
        await store.receive(\.sidebar.delegate.openInbox)
        await store.send(\.sidebar.projectDeleted, project.id)
        // Were its board opened all the same, it would open anew.
        await store.send(\.sidebar.delegate.openBoard, project)
        XCTAssertNotNil(store.state.board)
        XCTAssertNil(store.state.board?.actionFailure)
    }

    func testTheOpenSessionFollowsItsMoveToAnotherHost() async throws {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var moved = session
        moved.host = "far"
        var state = AppFeature.State()
        state.sidebar.sessions = [session]
        state.session = SessionFeature.State(session: session)
        let store = TestStore(initialState: state) { AppFeature() }
        store.exhaustivity = .off

        await store.send(\.sidebar.sessionMoved, moved)
        XCTAssertEqual(store.state.session?.session.host, "far")
    }

    func testTheOpenSessionShowsWhatTheSidebarReadsOfItsTaskAndArchiving() async throws {
        let session = Session(
            id: "s1", name: "first", backend: .scripted, workingDirectory: "/work", projectID: "p1",
            itemID: "item-1", itemTitle: "login")
        var listed = session
        listed.itemID = "item-2"
        listed.itemTitle = "logout"
        listed.archived = true
        listed.host = "far"
        var open = SessionFeature.State(session: session)
        open.tab = .terminal
        open.draft = "half written"
        var state = AppFeature.State()
        state.session = open
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
        }
        store.exhaustivity = .off

        await store.send(.sidebar(.loaded(projects: [], sessions: [listed], inbox: [])))
        XCTAssertEqual(store.state.session?.session, listed)
        // An archived session has no terminal to show.
        XCTAssertEqual(store.state.session?.tab, .chat)
        XCTAssertEqual(store.state.session?.draft, "half written")
    }

    func testTheOpenSessionFollowsTheCommitsItsLastTurnReadAsTheSidebarListsThem() async throws {
        var session = Session(
            id: "s1", name: "plan", backend: .scripted, workingDirectory: "/work", projectID: "p1")
        session.codeCommits = [CodeCommit(repository: "humata", commit: "aaaa1111")]
        var listed = session
        listed.codeCommits = [CodeCommit(repository: "humata", commit: "bbbb2222")]
        var state = AppFeature.State()
        state.session = SessionFeature.State(session: session)
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
        }
        store.exhaustivity = .off

        await store.send(.sidebar(.loaded(projects: [], sessions: [listed], inbox: [])))
        XCTAssertEqual(store.state.session?.session.codeCommits, listed.codeCommits)
    }

    func testANewerHumatasDaemonIsLeftRunningAndSaidSo() async throws {
        let restarts = LockIsolated(0)
        let starts = LockIsolated(0)
        let store = TestStore(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
            $0.daemonService = DaemonService(
                start: { starts.withValue { $0 += 1 } }, build: { .newer },
                restart: { restarts.withValue { $0 += 1 } }, openLoginItems: {}, stop: { false })
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.newerDaemonFound)
        await store.finish()
        XCTAssertEqual(store.state.daemon, .newerRunning)
        XCTAssertEqual(restarts.value, 0)
        // This copy's login item isn't put over the newer one's either.
        XCTAssertEqual(starts.value, 0)
    }

    func testOpeningATaskOnABoardKeptShowsTheBoardAsItWasLeft() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let item = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        var board = BoardFeature.State(project: project)
        board.actionFailure = "The card couldn't be moved."
        var state = AppFeature.State()
        state.sidebar.projects = [project]
        state.sidebar.selection = .board(project.id)
        state.board = board
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.selected, .search)
        await store.receive(\.sidebar.delegate.openSearch)
        XCTAssertNil(store.state.board)
        await store.send(\.search.delegate.openItem, (projectID: project.id, itemID: item.id))
        XCTAssertEqual(store.state.board?.actionFailure, "The card couldn't be moved.")
        XCTAssertEqual(store.state.board?.opening, item.id)

        // It's read at once, and the task's page opens.
        await store.receive(\.board.cardTapped)
        guard case .item(let page) = store.state.board?.destination else {
            return XCTFail("the task's page isn't open")
        }
        XCTAssertEqual(page.item.id, item.id)
    }

    func testAMessageThatFailsAfterLookingAwayGoesBackInItsSessionsBox() async throws {
        let first = Session(id: "unsent-first", name: "first", backend: .scripted, workingDirectory: "/work")
        let second = Session(id: "unsent-second", name: "second", backend: .scripted, workingDirectory: "/work")
        let image = DraftImage(
            id: "i1", name: "shot.png", image: ImageAttachment(mediaType: "image/png", data: Data([1])))
        var open = SessionFeature.State(session: first)
        open.readsImages = true
        open.images = [image]
        var state = AppFeature.State()
        state.sidebar.sessions = [first, second]
        state.sidebar.selection = .session(first.id)
        state.session = open
        // The send is still under way when the other session opens.
        let (answered, answer) = AsyncStream<Void>.makeStream()
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.daemon.sendImages = { _, _, _, _ in
                for await _ in answered { break }
                throw DaemonError.disconnected
            }
            $0.daemon.chatCommands = { _ in ChatCommands() }
        }
        store.exhaustivity = .off

        await store.send(\.session.binding.draft, "ship it")
        await store.send(\.session.sendTapped, .enqueue)
        await store.receive(\.session.delegate.send)
        await store.send(\.sidebar.selected, .session(second.id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.session.id, second.id)

        answer.yield()
        await store.receive(\.sendFailed)
        XCTAssertNotNil(store.state.sendFailure, "it says which session's message wasn't sent")
        XCTAssertEqual(store.state.session?.draft, "", "the session now open is left alone")
        XCTAssertEqual(SessionFeature.State(session: first).draft, "ship it")

        // The alert opens the session, with the message back in its box.
        await store.send(.sendFailure(.presented(.openSession(first.id))))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.session.id, first.id)
        XCTAssertEqual(store.state.session?.draft, "ship it")
        XCTAssertEqual(store.state.session?.images.map(\.id), [image.id])
        XCTAssertNil(store.state.sendFailure)
    }

    func testAMessageThatFailsWhileItsSessionIsOpenGoesBackInItsBox() async throws {
        let session = Session(id: "unsent-open", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = AppFeature.State()
        state.sidebar.sessions = [session]
        state.session = SessionFeature.State(session: session)
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.daemon.send = { _, _, _ in throw DaemonError.disconnected }
        }
        store.exhaustivity = .off

        await store.send(\.session.binding.draft, "ship it")
        await store.send(\.session.sendTapped, .enqueue)
        await store.receive(\.session.sendFailed)
        XCTAssertEqual(store.state.session?.draft, "ship it")
        XCTAssertEqual(store.state.session?.sending, 0)
        XCTAssertNotNil(store.state.session?.alert)
        XCTAssertNil(store.state.sendFailure)
    }

    func testAMessageToASessionDeletedSinceIsNotKept() async throws {
        let session = Session(id: "unsent-deleted", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = AppFeature.State()
        state.sidebar.sessions = [session]
        state.sidebar.selection = .session(session.id)
        state.session = SessionFeature.State(session: session)
        let (answered, answer) = AsyncStream<Void>.makeStream()
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.daemon.send = { _, _, _ in
                for await _ in answered { break }
                throw DaemonError.disconnected
            }
        }
        store.exhaustivity = .off

        await store.send(\.session.binding.draft, "ship it")
        await store.send(\.session.sendTapped, .enqueue)
        await store.receive(\.session.delegate.send)
        await store.send(\.sidebar.sessionDeleted, session.id)
        answer.yield()
        await store.receive(\.sendFailed)
        XCTAssertNil(store.state.sendFailure)
        XCTAssertEqual(SessionFeature.State(session: session).draft, "")
    }

    func testAMessageShowsAsBeingSentUntilTheDaemonHasIt() async throws {
        let session = Session(id: "sending", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = AppFeature.State()
        state.sidebar.sessions = [session]
        state.session = SessionFeature.State(session: session)
        let sent = LockIsolated<[String]>([])
        let (answered, answer) = AsyncStream<Void>.makeStream()
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.daemon.send = { _, text, _ in
                for await _ in answered { break }
                sent.withValue { $0.append(text) }
            }
        }
        store.exhaustivity = .off

        await store.send(\.session.binding.draft, "ship it")
        await store.send(\.session.sendTapped, .enqueue)
        await store.receive(\.session.delegate.send)
        XCTAssertEqual(store.state.session?.sending, 1)

        answer.yield()
        await store.receive(\.session.sent)
        XCTAssertEqual(store.state.session?.sending, 0)
        XCTAssertEqual(sent.value, ["ship it"])
    }

    func testDeletingASessionForgetsWhatWasBeingWrittenToIt() async throws {
        let session = Session(id: "deleted-draft", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = AppFeature.State()
        state.sidebar.sessions = [session]
        state.sidebar.selection = .session(session.id)
        state.session = SessionFeature.State(session: session)
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off

        await store.send(\.session.binding.draft, "half written")
        await store.send(\.session.binding.draft, "half written" + String(repeating: "x", count: 2_000))
        XCTAssertEqual(SessionFeature.State(session: session).draft, "half written")
        XCTAssertEqual(SessionFeature.State(session: session).pastes.count, 1)

        await store.send(\.sidebar.sessionDeleted, session.id)
        XCTAssertEqual(SessionFeature.State(session: session).draft, "")
        XCTAssertEqual(SessionFeature.State(session: session).pastes, [])
    }

    private func event(_ id: String, _ kind: SessionEvent.Kind) -> SessionEvent {
        SessionEvent(id: id, time: Date(timeIntervalSince1970: 0), kind: kind)
    }

    /// A store over `sessions` whose daemon holds `histories`, read as they
    /// are when a watch starts, and sends a watcher what follows the event
    /// it has, or all of it when it holds that event no longer. `asked`
    /// takes the event each watch said it has. `beside`, if given, is
    /// open from the start as a side chat of the first of `sessions`.
    private func store(
        _ sessions: [Session], _ histories: LockIsolated<[Session.ID: [SessionEvent]]>,
        asked: LockIsolated<[String?]>, beside: Session? = nil
    ) -> TestStoreOf<AppFeature> {
        var state = AppFeature.State()
        state.sidebar.sessions = IdentifiedArray(uniqueElements: sessions)
        if let beside, let first = sessions.first {
            var side = SessionFeature.State(session: beside)
            side.beside = true
            state.sidebar.selection = .session(first.id)
            state.session = SessionFeature.State(session: first)
            state.side = side
        }
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.daemon.watch = { session, after in
                asked.withValue { $0.append(after) }
                let history = histories.value[session] ?? []
                let has = history.firstIndex { $0.id == after }.map { $0 + 1 } ?? 0
                return AsyncThrowingStream { continuation in
                    for event in history[has...] { continuation.yield(.event(event)) }
                    continuation.yield(.historyLoaded)
                }
            }
            $0.daemon.markSeen = { _, _ in }
            $0.daemon.chatCommands = { _ in ChatCommands() }
        }
        store.exhaustivity = .off
        return store
    }

    func testAChatOpenedAgainShowsAtOnceAndIsWatchedFromItsLastEvent() async throws {
        let first = Session(id: "kept-first", name: "first", backend: .scripted, workingDirectory: "/work")
        let second = Session(id: "kept-second", name: "second", backend: .scripted, workingDirectory: "/work")
        let history = [
            event("e1", .userMessage("hi")), event("e2", .turnStarted), event("e3", .assistantMessage("hello")),
        ]
        let asked = LockIsolated<[String?]>([])
        let store = store([first, second], LockIsolated([first.id: history]), asked: asked)

        await store.send(\.sidebar.selected, .session(first.id))
        await store.receive(\.sidebar.delegate.openSession)
        await store.send(\.session.task)
        await store.receive(\.session.receivedAll, timeout: .seconds(1))
        XCTAssertEqual(store.state.session?.items.map(\.id), ["e1", "e3"])

        await store.send(\.sidebar.selected, .session(second.id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.items, [])

        // Back to the first: its rows are there before anything is read.
        await store.send(\.sidebar.selected, .session(first.id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.historyLoaded, true)
        XCTAssertEqual(store.state.session?.items.map(\.id), ["e1", "e3"])
        XCTAssertEqual(store.state.session?.isWorking, true)

        await store.send(\.session.task)
        await store.receive(\.session.receivedAll, timeout: .seconds(1))
        XCTAssertEqual(asked.value, [nil, "e3"])
        XCTAssertEqual(store.state.session?.events.map(\.id), ["e1", "e2", "e3"])
        await store.skipInFlightEffects()
    }

    func testAChatTooLargeToKeepIsReadAgainWhenItOpensAgain() async throws {
        let large = Session(id: "kept-large", name: "large", backend: .scripted, workingDirectory: "/work")
        let other = Session(id: "kept-other", name: "other", backend: .scripted, workingDirectory: "/work")
        // More text than is kept of every chat together.
        let history = [event("b1", .assistantMessage(String(repeating: "x", count: 49 * 1_048_576)))]
        let asked = LockIsolated<[String?]>([])
        let store = store([large, other], LockIsolated([large.id: history]), asked: asked)

        await store.send(\.sidebar.selected, .session(large.id))
        await store.receive(\.sidebar.delegate.openSession)
        await store.send(\.session.task)
        await store.receive(\.session.receivedAll, timeout: .seconds(5))
        XCTAssertEqual(store.state.session?.items.map(\.id), ["b1"])

        await store.send(\.sidebar.selected, .session(other.id))
        await store.receive(\.sidebar.delegate.openSession)
        await store.send(\.sidebar.selected, .session(large.id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.historyLoaded, false)
        XCTAssertEqual(store.state.session?.items, [])

        await store.send(\.session.task)
        await store.receive(\.session.receivedAll, timeout: .seconds(5))
        XCTAssertEqual(asked.value, [nil, nil])
        XCTAssertEqual(store.state.session?.items.map(\.id), ["b1"])
        await store.skipInFlightEffects()
    }

    func testOnlyTheChatsLastLookedAwayFromAreKept() async throws {
        let sessions = (1...6).map {
            Session(id: "kept-\($0)", name: "chat \($0)", backend: .scripted, workingDirectory: "/work")
        }
        let histories = Dictionary(
            uniqueKeysWithValues: sessions.map { ($0.id, [event("\($0.id)-e1", .userMessage("hi"))]) })
        let store = store(sessions, LockIsolated(histories), asked: LockIsolated([]))

        // Each is opened and read, the last still open.
        for session in sessions {
            await store.send(\.sidebar.selected, .session(session.id))
            await store.receive(\.sidebar.delegate.openSession)
            await store.send(\.session.task)
            await store.receive(\.session.receivedAll, timeout: .seconds(1))
        }

        // The one before it shows at once; the first, left longest ago,
        // is read again.
        await store.send(\.sidebar.selected, .session(sessions[4].id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.items.map(\.id), ["kept-5-e1"])
        await store.send(\.sidebar.selected, .session(sessions[0].id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.historyLoaded, false)
        XCTAssertEqual(store.state.session?.items, [])
        await store.skipInFlightEffects()
    }

    func testAChatRewoundWhileItWasAwayIsReadAgainFromTheStart() async throws {
        let first = Session(id: "kept-rewound", name: "first", backend: .scripted, workingDirectory: "/work")
        let second = Session(id: "kept-beside", name: "second", backend: .scripted, workingDirectory: "/work")
        let history = [
            event("e1", .userMessage("one")), event("e2", .assistantMessage("two")), event("e3", .userMessage("three")),
        ]
        let histories = LockIsolated([first.id: history])
        let asked = LockIsolated<[String?]>([])
        let store = store([first, second], histories, asked: asked)

        await store.send(\.sidebar.selected, .session(first.id))
        await store.receive(\.sidebar.delegate.openSession)
        await store.send(\.session.task)
        await store.receive(\.session.receivedAll, timeout: .seconds(1))
        await store.send(\.sidebar.selected, .session(second.id))
        await store.receive(\.sidebar.delegate.openSession)

        // Rewound from elsewhere meanwhile: the history no longer holds
        // the last event the chat has.
        let rewound = [history[0], history[1], event("r1", .rewound(text: "three", files: false))]
        histories.withValue { $0[first.id] = rewound }

        await store.send(\.sidebar.selected, .session(first.id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.events.map(\.id), ["e1", "e2", "e3"])
        await store.send(\.session.task)
        await store.receive(\.session.receivedAll, timeout: .seconds(1))
        // It starts over, and reads what the rewind left.
        await store.receive(\.session.task, timeout: .seconds(1))
        await store.receive(\.session.receivedAll, timeout: .seconds(1))
        XCTAssertEqual(asked.value, [nil, "e3", nil])
        XCTAssertEqual(store.state.session?.events.map(\.id), ["e1", "e2", "r1"])
        await store.skipInFlightEffects()
    }

    /// A store with a session open whose agent asked to start another,
    /// over a daemon that takes each answer only once `gate` gives it an
    /// outcome: true lets the answer through, false fails it. `answers`
    /// counts the answers that reached the daemon.
    private func withAStartRequestOpen(
        gate: AsyncStream<Bool>, answers: LockIsolated<Int>
    ) async throws -> (TestStoreOf<AppFeature>, AgentRequest) {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let session = try await daemon.client.start(StartRequest(backend: .scripted, projectID: project.id))
        let request = daemon.addRequest(session.id, .startSession(name: "Tests"), "Write the tests.")
        let client = daemon.client
        let store = TestStore(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = client
            $0.daemon.answerRequest = { id, answer in
                answers.withValue { $0 += 1 }
                var goes = true
                for await outcome in gate {
                    goes = outcome
                    break
                }
                guard goes else { throw DaemonError.disconnected }
                try await client.answerRequest(id, answer)
            }
            $0.notifications.post = { _ in }
        }
        store.exhaustivity = .off
        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loaded)
        await store.send(\.sidebar.selected, .session(session.id))
        await store.receive(\.sidebar.delegate.openSession)
        return (store, request)
    }

    func testAllowingARequestTwiceFromTheStripAnswersItOnce() async throws {
        let (gate, outcome) = AsyncStream<Bool>.makeStream()
        let answers = LockIsolated(0)
        let (store, request) = try await withAStartRequestOpen(gate: gate, answers: answers)
        XCTAssertEqual(store.state.session?.request?.id, request.id)

        await store.send(\.session.requestAllowed)
        await store.receive(\.sidebar.requestAnswered)
        XCTAssertEqual(store.state.session?.answeringRequest, true)
        XCTAssertEqual(store.state.session?.request?.id, request.id, "the strip stays up while it's answered")

        await store.send(\.session.requestAllowed)
        await Task.megaYield()
        XCTAssertEqual(answers.value, 1)

        outcome.yield(true)
        await store.receive(\.sidebar.requestAnswerSent)
        XCTAssertNil(store.state.session?.request)
        XCTAssertEqual(store.state.session?.answeringRequest, false)
        await store.skipInFlightEffects()
    }

    func testAListReadWhileARequestIsBeingAnsweredDoesNotOfferItAgain() async throws {
        let (gate, outcome) = AsyncStream<Bool>.makeStream()
        let answers = LockIsolated(0)
        let (store, request) = try await withAStartRequestOpen(gate: gate, answers: answers)

        await store.send(\.session.requestAllowed)
        await store.receive(\.sidebar.requestAnswered)
        // The daemon still lists the request: its answer hasn't landed.
        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loaded)
        XCTAssertEqual(store.state.session?.answeringRequest, true)
        XCTAssertEqual(store.state.session?.request?.id, request.id)

        outcome.yield(true)
        await store.receive(\.sidebar.requestAnswerSent)
        // Nor does a read from before the answer, once it has.
        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loaded)
        XCTAssertNil(store.state.session?.request)
        XCTAssertEqual(answers.value, 1)
        await store.skipInFlightEffects()
    }

    func testARequestWhoseAnswerFailedCanBeAnsweredAgainFromTheStrip() async throws {
        let (gate, outcome) = AsyncStream<Bool>.makeStream()
        let answers = LockIsolated(0)
        let (store, request) = try await withAStartRequestOpen(gate: gate, answers: answers)

        await store.send(\.session.requestAllowed)
        await store.receive(\.sidebar.requestAnswered)
        outcome.yield(false)
        await store.receive(\.sidebar.requestNotAnswered)
        XCTAssertEqual(store.state.session?.answeringRequest, false)
        XCTAssertNil(store.state.session?.requestAnswer, "the answer that failed is no longer the one shown")
        XCTAssertEqual(store.state.session?.request?.id, request.id)

        await store.send(\.session.requestAllowed)
        await store.receive(\.sidebar.requestAnswered)
        XCTAssertEqual(store.state.session?.answeringRequest, true)
        XCTAssertEqual(store.state.session?.requestAnswer, .allowing)
        outcome.yield(true)
        await store.receive(\.sidebar.requestAnswerSent)
        XCTAssertEqual(answers.value, 2)
        XCTAssertNil(store.state.session?.request)
        await store.skipInFlightEffects()
    }

    func testASessionOpenedFromASearchHitShowsWhatItsAgentAsks() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", name: "first"))
        try await daemon.client.send(session.id, "the migrations fail", .enqueue)
        let message = try XCTUnwrap(daemon.history(session.id).first { $0.kind == .userMessage("the migrations fail") })
        let asked = daemon.addRequest(session.id, .question, "Which of the two?")
        let store = TestStore(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.notifications.post = { _ in }
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loaded)
        await store.send(\.sidebar.selected, .search)
        await store.receive(\.sidebar.delegate.openSearch)
        await store.send(\.search.delegate.openSession, (id: session.id, event: message.id))
        XCTAssertEqual(store.state.session?.session.id, session.id)
        XCTAssertEqual(store.state.session?.request?.id, asked.id)
        await store.skipInFlightEffects()
    }

    func testAnEditSentFromTheSideChatOpensItsForkBesideTheOpenSession() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", name: "first"))
        try await daemon.client.send(session.id, "hello", .enqueue)
        let store = try await withASideChat(daemon, of: session, question: "and why?")
        let side = try XCTUnwrap(store.state.side?.session)
        let asked = try XCTUnwrap(daemon.history(side.id).first { $0.kind == .userMessage("and why?") })

        await store.send(\.side.task)
        await store.receive(\.side.receivedAll, timeout: .seconds(1))
        await store.send(\.side.editTapped, asked.id)
        await store.send(\.side.binding.edited, "and how?")
        await store.send(\.side.editSent)
        await store.receive(\.side.delegate.openSession, timeout: .seconds(1))

        XCTAssertEqual(store.state.session?.session.id, session.id, "the open session stays as it is")
        XCTAssertEqual(store.state.sidebar.selection, .session(session.id))
        let fork = try XCTUnwrap(store.state.side?.session)
        XCTAssertNotEqual(fork.id, side.id)
        XCTAssertEqual(fork.parentID, side.id)
        XCTAssertTrue(fork.readOnly)
        await store.skipInFlightEffects()
    }

    func testASideChatClosedAndOpenedFromTheSidebarShowsAtOnceAndIsWatchedFromItsLastEvent() async throws {
        let first = Session(id: "side-kept-first", name: "first", backend: .scripted, workingDirectory: "/work")
        var side = Session(id: "side-kept-side", name: "side", backend: .scripted, workingDirectory: "/work")
        side.parentID = first.id
        side.readOnly = true
        let history = [
            event("s1", .userMessage("why?")), event("s2", .turnStarted), event("s3", .assistantMessage("because")),
        ]
        let asked = LockIsolated<[String?]>([])
        let store = store([first, side], LockIsolated([side.id: history]), asked: asked, beside: side)

        await store.send(\.side.task)
        await store.receive(\.side.receivedAll, timeout: .seconds(1))
        XCTAssertEqual(store.state.side?.items.map(\.id), ["s1", "s3"])

        await store.send(.side(.dismiss))
        XCTAssertNil(store.state.side)

        await store.send(\.sidebar.selected, .session(side.id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.session.id, side.id)
        XCTAssertEqual(store.state.session?.historyLoaded, true)
        XCTAssertEqual(store.state.session?.items.map(\.id), ["s1", "s3"])

        await store.send(\.session.task)
        await store.receive(\.session.receivedAll, timeout: .seconds(1))
        XCTAssertEqual(asked.value, [nil, "s3"])
        await store.skipInFlightEffects()
    }

    func testASideChatReplacedByAnotherKeepsTheImageInItsBox() async throws {
        let first = Session(id: "replaced-first", name: "first", backend: .scripted, workingDirectory: "/work")
        var side = Session(id: "replaced-side", name: "side", backend: .scripted, workingDirectory: "/work")
        side.parentID = first.id
        side.readOnly = true
        var next = Session(id: "replaced-next", name: "next", backend: .scripted, workingDirectory: "/work")
        next.parentID = side.id
        next.readOnly = true
        let image = DraftImage(
            id: "i1", name: "shot.png", image: ImageAttachment(mediaType: "image/png", data: Data([1])))
        var beside = SessionFeature.State(session: side)
        beside.beside = true
        beside.images = [image]
        var state = AppFeature.State()
        state.sidebar.sessions = [first, side, next]
        state.sidebar.selection = .session(first.id)
        state.session = SessionFeature.State(session: first)
        state.side = beside
        let listed = [first, side, next]
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon.projects = { [] }
            $0.daemon.sessions = { _ in listed }
            $0.daemon.inbox = { Inbox() }
            $0.notifications.post = { _ in }
        }
        store.exhaustivity = .off

        await store.send(.columnFocused(.side))
        await store.send(\.side.delegate.openSide, next)
        await store.receive(\.sidebar.loaded)
        XCTAssertEqual(store.state.side?.session.id, next.id)
        XCTAssertEqual(store.state.side?.images, [])
        XCTAssertEqual(store.state.focusedColumn, .side, "the person stays in the column they were in")
        XCTAssertEqual(store.state.side?.takesShortcuts, true)

        await store.send(\.sidebar.selected, .session(side.id))
        await store.receive(\.sidebar.delegate.openSession)
        XCTAssertEqual(store.state.session?.session.id, side.id)
        XCTAssertEqual(store.state.session?.images.map(\.id), [image.id])
        await store.skipInFlightEffects()
    }

    func testClosingTheSideChatFromItsColumnPutsTheCaretInTheSessionsBox() async throws {
        let first = Session(id: "caret-first", name: "first", backend: .scripted, workingDirectory: "/work")
        var side = Session(id: "caret-side", name: "side", backend: .scripted, workingDirectory: "/work")
        side.parentID = first.id
        side.readOnly = true

        // From the side column, the box of the session left takes it.
        let inSide = besideEachOther(first, side, stopped: LockIsolated([]))
        let before = inSide.state.session?.boxFocus
        await inSide.send(.columnFocused(.side))
        await inSide.send(.side(.dismiss))
        XCTAssertNil(inSide.state.side)
        XCTAssertNotEqual(inSide.state.session?.boxFocus, before)

        // From the main column, the caret is where the person left it.
        let inMain = besideEachOther(first, side, stopped: LockIsolated([]))
        await inMain.send(.side(.dismiss))
        XCTAssertNil(inMain.state.side)
        XCTAssertEqual(inMain.state.session?.boxFocus, before)
    }

    func testRewindFromTheMenuOpensThePickerOfTheColumnInUse() async throws {
        let first = Session(id: "menu-first", name: "first", backend: .scripted, workingDirectory: "/work")
        var side = Session(id: "menu-side", name: "side", backend: .scripted, workingDirectory: "/work")
        side.parentID = first.id
        side.readOnly = true
        let history = [event("m1", .userMessage("why?")), event("m2", .assistantMessage("because"))]
        let store = store(
            [first, side], LockIsolated([first.id: history, side.id: history]), asked: LockIsolated([]),
            beside: side)
        await store.send(\.session.task)
        await store.receive(\.session.receivedAll, timeout: .seconds(1))
        await store.send(\.side.task)
        await store.receive(\.side.receivedAll, timeout: .seconds(1))

        await store.send(.columnFocused(.side))
        await store.send(.sessionCommand(.rewindPickerAsked))
        await store.receive(\.side.rewindPickerAsked)
        XCTAssertNotNil(store.state.side?.rewindPicker)
        XCTAssertNil(store.state.session?.rewindPicker, "the open session's chat is left alone")
        await store.skipInFlightEffects()
    }

    /// A daemon service whose daemon answers or not as `gone` says, and
    /// counts how often it was started.
    private func service(
        gone: LockIsolated<Bool>, starts: LockIsolated<Int>, start: @escaping @Sendable () throws -> Void = {}
    ) -> DaemonService {
        DaemonService(
            start: {
                starts.withValue { $0 += 1 }
                try start()
            },
            build: {
                if gone.value { throw DaemonError.notRunning(socket: "/tmp/h.sock", reason: "refused") }
                return .current
            },
            restart: {}, openLoginItems: {}, stop: { false })
    }

    func testASidebarReadThatFailsWithTheDaemonGoneStartsItAgain() async throws {
        let gone = LockIsolated(true)
        let starts = LockIsolated(0)
        var state = AppFeature.State()
        state.daemon = .running
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
            let projects = $0.daemon.projects
            $0.daemon.projects = {
                if gone.value { throw DaemonError.notRunning(socket: "/tmp/h.sock", reason: "refused") }
                return try await projects()
            }
            $0.daemonService = service(gone: gone, starts: starts) { gone.setValue(false) }
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loadFailed)
        await store.receive(\.daemonGone)
        XCTAssertEqual(store.state.daemon, .restarting)
        await store.receive(\.restarted)
        XCTAssertEqual(store.state.daemon, .running)
        XCTAssertEqual(starts.value, 1)
        // Started again, it's read again.
        await store.receive(\.sidebar.loaded)
    }

    func testAReadThatFailsWhileTheDaemonAnswersStartsNothing() async throws {
        let starts = LockIsolated(0)
        var state = AppFeature.State()
        state.daemon = .running
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
            $0.daemon.projects = { throw DaemonError.unexpected("a list it couldn't make") }
            $0.daemonService = service(gone: LockIsolated(false), starts: starts)
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loadFailed)
        await store.finish()
        XCTAssertEqual(store.state.daemon, .running)
        XCTAssertEqual(starts.value, 0)
    }

    func testTheDaemonIsStartedAgainOnceUntilTheSidebarReadsAgain() async throws {
        let gone = LockIsolated(true)
        let starts = LockIsolated(0)
        var state = AppFeature.State()
        state.daemon = .running
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
            let projects = $0.daemon.projects
            $0.daemon.projects = {
                if gone.value { throw DaemonError.notRunning(socket: "/tmp/h.sock", reason: "refused") }
                return try await projects()
            }
            // Starting it doesn't bring it back.
            $0.daemonService = service(gone: gone, starts: starts)
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.refresh)
        await store.receive(\.restarted)
        // The read after the start fails too, and so does one more.
        await store.finish()
        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loadFailed)
        await store.finish()
        XCTAssertEqual(starts.value, 1)

        // Once the sidebar has read, a daemon that goes is started again.
        gone.setValue(false)
        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loaded)
        gone.setValue(true)
        await store.send(\.sidebar.refresh)
        await store.receive(\.restarted)
        await store.finish()
        XCTAssertEqual(starts.value, 2)
    }

    func testADaemonGoneWhileAnUpdateWaitsIsStartedAgain() async throws {
        let gone = LockIsolated(true)
        let starts = LockIsolated(0)
        var state = AppFeature.State()
        state.daemon = .updatePending
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
            let projects = $0.daemon.projects
            $0.daemon.projects = {
                if gone.value { throw DaemonError.notRunning(socket: "/tmp/h.sock", reason: "refused") }
                return try await projects()
            }
            $0.daemonService = service(gone: gone, starts: starts) { gone.setValue(false) }
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loadFailed)
        await store.receive(\.daemonGone)
        await store.receive(\.restarted)
        // The one started is this app's, so no update waits any more.
        XCTAssertEqual(store.state.daemon, .running)
        XCTAssertEqual(starts.value, 1)
        await store.receive(\.sidebar.loaded)
    }

    func testADaemonThatAnsweredOnceIsLookedForAgainWhenReadsKeepFailing() async throws {
        let gone = LockIsolated(false)
        let failing = LockIsolated(true)
        let starts = LockIsolated(0)
        var state = AppFeature.State()
        state.daemon = .running
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
            let projects = $0.daemon.projects
            $0.daemon.projects = {
                if failing.value { throw DaemonError.unexpected("a list it couldn't make") }
                return try await projects()
            }
            $0.daemonService = service(gone: gone, starts: starts) {
                gone.setValue(false)
                failing.setValue(false)
            }
        }
        store.exhaustivity = .off

        // A read fails while the daemon answers: nothing is started.
        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loadFailed)
        await store.finish()
        XCTAssertEqual(store.state.daemon, .running)
        XCTAssertEqual(starts.value, 0)

        // Then the daemon goes, with no read having succeeded in between.
        gone.setValue(true)
        await store.send(\.sidebar.refresh)
        await store.receive(\.restarted)
        XCTAssertEqual(store.state.daemon, .running)
        XCTAssertEqual(starts.value, 1)
        await store.receive(\.sidebar.loaded)
    }

    func testADaemonStartedAgainWhoseReadsStillFailIsStartedAgainWhenItGoes() async throws {
        let gone = LockIsolated(true)
        let failing = LockIsolated(true)
        let starts = LockIsolated(0)
        var state = AppFeature.State()
        state.daemon = .running
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
            let projects = $0.daemon.projects
            $0.daemon.projects = {
                if failing.value { throw DaemonError.unexpected("a list it couldn't make") }
                return try await projects()
            }
            // Started, it answers; the second time its reads work too.
            $0.daemonService = service(gone: gone, starts: starts) {
                gone.setValue(false)
                if starts.value > 1 { failing.setValue(false) }
            }
        }
        store.exhaustivity = .off

        // Gone, it's started and answers, and the read after it still fails.
        await store.send(\.sidebar.refresh)
        await store.receive(\.restarted)
        await store.finish()
        XCTAssertEqual(store.state.daemon, .running)
        XCTAssertEqual(starts.value, 1)

        // Then it goes, with no read having succeeded in between.
        gone.setValue(true)
        await store.send(\.sidebar.refresh)
        await store.receive(\.restarted)
        XCTAssertEqual(store.state.daemon, .running)
        XCTAssertEqual(starts.value, 2)
        await store.receive(\.sidebar.loaded)
    }

    func testARestartOfADaemonHumataDidntStartLeavesItRunningAndSaysSoInHosts() async throws {
        var state = AppFeature.State()
        state.daemon = .running
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
            $0.daemonService = DaemonService(
                start: {}, build: { .current },
                restart: { throw DaemonServiceError.notManaged(socket: "/tmp/h.sock") }, openLoginItems: {},
                stop: { false })
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.selected, .hosts)
        await store.receive(\.sidebar.delegate.openHosts)
        await store.send(\.hosts.restartDaemonTapped)
        await store.send(.hosts(.presented(.destination(.presented(.alert(.confirmRestartDaemon))))))
        await store.receive(\.hosts.delegate.restartDaemon)
        XCTAssertNil(store.state.hosts?.destination, "the question is answered")
        await store.receive(\.restartRefused)
        XCTAssertEqual(store.state.daemon, .running, "nothing was stopped")
        await store.receive(\.hosts.restartRefused)
        XCTAssertNotNil(store.state.hosts?.destination?.alert, "Hosts says why")
    }

    func testARefusedRestartThatWasntAskedForInHostsSaysWhyAboveTheWindow() async throws {
        var state = AppFeature.State()
        state.daemon = .updatePending
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
            $0.daemonService = DaemonService(
                start: {}, build: { .older },
                restart: { throw DaemonServiceError.notManaged(socket: "/tmp/h.sock") }, openLoginItems: {},
                stop: { false })
        }
        store.exhaustivity = .off

        await store.send(.restartTapped)
        await store.receive(\.restartRefused)
        guard case .failed(_, needsLoginItems: false) = store.state.daemon else {
            return XCTFail("expected it to say why it wasn't restarted, got \(store.state.daemon)")
        }
    }

    func testADaemonThatCantBeStartedAgainShowsWhy() async throws {
        let gone = LockIsolated(true)
        let starts = LockIsolated(0)
        let missing = LockIsolated(true)
        var state = AppFeature.State()
        state.daemon = .running
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
            let projects = $0.daemon.projects
            $0.daemon.projects = {
                if gone.value { throw DaemonError.notRunning(socket: "/tmp/h.sock", reason: "refused") }
                return try await projects()
            }
            $0.daemonService = service(gone: gone, starts: starts) {
                if missing.value { throw DaemonServiceError.missingExecutable(path: "/Applications/humatad") }
                gone.setValue(false)
            }
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.refresh)
        await store.receive(\.daemonFailed)
        guard case .failed(_, needsLoginItems: false) = store.state.daemon else {
            return XCTFail("expected it to say why it couldn't be started, got \(store.state.daemon)")
        }

        // Its Try Again starts it, once what kept it from starting is fixed.
        missing.setValue(false)
        await store.send(.task)
        await store.receive(\.daemonChecked)
        XCTAssertEqual(store.state.daemon, .running)
        XCTAssertEqual(starts.value, 2)
    }

    func testHostsAskingForARestartRestartsTheDaemon() async throws {
        let restarts = LockIsolated(0)
        var state = AppFeature.State()
        state.daemon = .running
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
            // The shell is read when the daemon starts, so only a new one
            // finds it fixed.
            $0.daemon.linkCommands = { _ in
                CommandLinks(
                    folder: "/Users/me/.local/bin", links: [], onPath: true,
                    environmentProblem: restarts.value == 0 ? "the shell didn't answer" : nil)
            }
            $0.daemonService = DaemonService(
                start: {}, build: { .current }, restart: { restarts.withValue { $0 += 1 } }, openLoginItems: {},
                stop: { false })
        }
        store.exhaustivity = .off

        await store.send(\.sidebar.selected, .hosts)
        await store.receive(\.sidebar.delegate.openHosts)
        await store.send(\.hosts.task)
        await store.receive(\.hosts.commandLinksLoaded)
        XCTAssertNotNil(store.state.hosts?.commandLinks?.environmentProblem)

        await store.send(\.hosts.restartDaemonTapped)
        XCTAssertEqual(restarts.value, 0)
        await store.send(.hosts(.presented(.destination(.presented(.alert(.confirmRestartDaemon))))))
        await store.receive(\.hosts.delegate.restartDaemon)
        XCTAssertEqual(store.state.daemon, .restarting)
        await store.receive(\.restarted)
        XCTAssertEqual(store.state.daemon, .running)
        XCTAssertEqual(restarts.value, 1)
        // Hosts reads what the new daemon found of the shell.
        await store.receive(\.hosts.commandLinksLoaded)
        XCTAssertNil(store.state.hosts?.commandLinks?.environmentProblem)
    }

    func testLinksThatNameNothingAreIgnored() throws {
        for text in ["humata://session", "humata://elsewhere/x", "https://session/x", "humata://task/only-a-project"] {
            XCTAssertNil(HumataLink(try XCTUnwrap(URL(string: text))), text)
        }
        XCTAssertEqual(HumataLink(try XCTUnwrap(URL(string: "humata://board/p1"))), .board("p1"))
    }
}
