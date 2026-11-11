import ComposableArchitecture
import HumataAppFeature
import HumataDaemonClient
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
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        daemon.setOffline("far", true)
        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loaded)
        XCTAssertEqual(store.state.session?.session.hostOffline, true)
    }
}
