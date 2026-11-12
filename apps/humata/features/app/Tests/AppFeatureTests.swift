import ComposableArchitecture
import HumataAppFeature
import HumataDaemonClient
import HumataProjects
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

    func testLinksThatNameNothingAreIgnored() throws {
        for text in ["humata://session", "humata://elsewhere/x", "https://session/x", "humata://task/only-a-project"] {
            XCTAssertNil(HumataLink(try XCTUnwrap(URL(string: text))), text)
        }
        XCTAssertEqual(HumataLink(try XCTUnwrap(URL(string: "humata://board/p1"))), .board("p1"))
    }
}
