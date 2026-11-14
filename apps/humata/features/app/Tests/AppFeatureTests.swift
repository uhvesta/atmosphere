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

    func testLinksThatNameNothingAreIgnored() throws {
        for text in ["humata://session", "humata://elsewhere/x", "https://session/x", "humata://task/only-a-project"] {
            XCTAssertNil(HumataLink(try XCTUnwrap(URL(string: text))), text)
        }
        XCTAssertEqual(HumataLink(try XCTUnwrap(URL(string: "humata://board/p1"))), .board("p1"))
    }
}
