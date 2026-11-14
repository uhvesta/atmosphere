import ComposableArchitecture
import HumataDaemonClient
import HumataProjects
import XCTest

/// What agents ask to do to other sessions is allowed from the inbox, the
/// sessions they start show under them, and a project says what they may.
@MainActor
final class AgentSessionsTests: XCTestCase {
    func testASessionAnAgentAskedForIsAllowedFromTheInboxAndShowsUnderItsParent() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let parent = try await daemon.client.start(StartRequest(backend: .scripted, projectID: project.id))
        let other = try await daemon.client.start(StartRequest(backend: .scripted, projectID: project.id))
        let request = daemon.addRequest(parent.id, .startSession(name: "Tests"), "Write the tests.")
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.notifications.post = { _ in }
        }
        store.exhaustivity = .off

        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.inbox.compactMap(\.request), [request])
        // ⇧⌘A allows it from where it is.
        XCTAssertEqual(store.state.inboxShortcut, .request(entry: request.id, request.id))
        XCTAssertNil(store.state.tree(in: project.id).first { $0.id == parent.id }?.children)

        await store.send(.requestAnswered(request.id, .approve))
        // Nothing to close, so nothing is asked first.
        XCTAssertNil(store.state.destination)
        await store.receive(\.loaded)
        XCTAssertFalse(store.state.inbox.contains { $0.request != nil })

        let tree = store.state.tree(in: project.id)
        XCTAssertEqual(Set(tree.map(\.id)), [parent.id, other.id])
        let children = try XCTUnwrap(tree.first { $0.id == parent.id }?.children)
        XCTAssertEqual(children.map(\.session.name), ["Tests"])
        XCTAssertNil(tree.first { $0.id == other.id }?.children)
    }

    func testDecliningWhatAnAgentAskedForStartsNothing() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let parent = try await daemon.client.start(StartRequest(backend: .scripted, projectID: project.id))
        let request = daemon.addRequest(parent.id, .startSession(name: "Tests"), "Write the tests.")
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.notifications.post = { _ in }
        }
        store.exhaustivity = .off

        await store.send(.refresh)
        await store.receive(\.loaded)
        await store.send(.requestAnswered(request.id, .dismiss))
        await store.receive(\.loaded)
        XCTAssertTrue(store.state.inbox.isEmpty)
        XCTAssertEqual(store.state.sessions.map(\.id), [parent.id])
    }

    func testSessionsStartedByAgentsNestAsDeepAsTheyWereStartedBesideForks() {
        let project = Project(id: "p", name: "code", repositories: [], created: Date(timeIntervalSince1970: 0))
        func session(_ id: String, startedBy: String? = nil, forkOf: String? = nil) -> Session {
            var session = Session(
                id: id, name: id, backend: .scripted, workingDirectory: "/work", parentID: forkOf, projectID: "p")
            session.startedBy = startedBy
            return session
        }
        var state = ProjectsFeature.State()
        state.projects = [project]
        state.sessions = [
            session("root"),
            session("child", startedBy: "root"),
            session("grandchild", startedBy: "child"),
            session("fork", forkOf: "root"),
            // Its starter is gone, so it shows at the top.
            session("orphan", startedBy: "deleted"),
        ]

        let tree = state.tree(in: "p")
        XCTAssertEqual(Set(tree.map(\.id)), ["root", "orphan"])
        let root = tree.first { $0.id == "root" }
        XCTAssertEqual(Set(root?.children?.map(\.id) ?? []), ["child", "fork"])
        let child = root?.children?.first { $0.id == "child" }
        XCTAssertEqual(child?.children?.map(\.id), ["grandchild"])
    }

    func testProjectSettingsSaveWhatAgentsMayDoOnlyWhenChanged() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        // Until the person chooses, each start and message asks.
        XCTAssertEqual(project.agentSessions.mode, .ask)
        XCTAssertFalse(project.agentSessions.messageOthers)

        let store = TestStore(initialState: ProjectSettingsFeature.State(project: project)) {
            ProjectSettingsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        let chosen = AgentSessions(mode: .allow, messageOthers: true, maxChildren: 2, maxDepth: 1)
        await store.send(\.binding.agentSessions, chosen)
        await store.send(.saveTapped)
        await store.receive(\.delegate.saved)
        let projects = try await daemon.client.projects()
        let saved = try XCTUnwrap(projects.first)
        XCTAssertEqual(saved.agentSessions, chosen)

        let sent = LockIsolated<ProjectUpdate?>(nil)
        let untouched = TestStore(initialState: ProjectSettingsFeature.State(project: saved)) {
            ProjectSettingsFeature()
        } withDependencies: {
            $0.daemon.updateProject = { _, update in
                sent.setValue(update)
                return saved
            }
        }
        untouched.exhaustivity = .off
        await untouched.send(\.binding.instructions, "be brief")
        await untouched.send(.saveTapped)
        await untouched.receive(\.delegate.saved)
        XCTAssertNotNil(sent.value)
        XCTAssertNil(sent.value?.agentSessions)
    }
}
