import ComposableArchitecture
import KhodrDaemonClient
import KhodrProjects
import XCTest

@MainActor
final class ProjectsFeatureTests: XCTestCase {
    func testAddingAProjectThenStartingASessionInItOpensIt() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.addProjectTapped)
        await store.send(
            \.destination.newProject.folderPicked, URL(filePath: "/code/khodr", directoryHint: .isDirectory))
        XCTAssertEqual(store.state.destination?.newProject?.name, "khodr")
        await store.send(\.destination.newProject.createTapped)
        await store.receive(\.destination.newProject.delegate.created)
        let project = try XCTUnwrap(store.state.projects.first)
        XCTAssertEqual(project.repository, "/code/khodr")
        XCTAssertEqual(store.state.destination?.newSession?.project, project)

        await store.send(\.destination.newSession.binding.backend, .scripted)
        await store.send(\.destination.newSession.binding.approvalMode, .allowEdits)
        await store.send(\.destination.newSession.startTapped)
        await store.receive(\.destination.newSession.delegate.started)
        let session = try XCTUnwrap(daemon.sessions.first)
        await store.receive(\.delegate.openSession, session)
        XCTAssertNil(store.state.destination)
        XCTAssertEqual(store.state.selection, session.id)
        XCTAssertEqual(store.state.sessions(in: project.id), [session])
        XCTAssertEqual(session.workingDirectory, "/code/khodr")
        XCTAssertEqual(session.approvalMode, .allowEdits)
        XCTAssertEqual(session.backend, .scripted)
    }

    func testAProjectNeedsANameAndAnAbsoluteFolder() {
        var state = NewProjectFeature.State()
        state.name = "khodr"
        state.repository = "code/khodr"
        XCTAssertFalse(state.canCreate)
        state.repository = "/code/khodr"
        XCTAssertTrue(state.canCreate)
        state.name = "  "
        XCTAssertFalse(state.canCreate)
    }

    func testARefusedProjectSaysWhyAndCanBeTriedAgain() async {
        let refusal = DaemonError.refused(.invalidRequest, message: "There is no folder /code/gone.")
        var state = NewProjectFeature.State()
        state.name = "gone"
        state.repository = "/code/gone"
        let store = TestStore(initialState: state) {
            NewProjectFeature()
        } withDependencies: {
            $0.daemon.createProject = { _, _, _ in throw refusal }
        }
        await store.send(.createTapped) {
            $0.creating = true
        }
        await store.receive(\.failed) {
            $0.creating = false
            $0.alert = AlertState {
                TextState("Couldn't add the project")
            } message: {
                TextState(refusal.description)
            }
        }
        XCTAssertTrue(store.state.canCreate)
    }

    func testTheListIsReadAgainUntilTheSidebarGoes() async throws {
        let daemon = InMemoryDaemon()
        let clock = TestClock()
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.continuousClock = clock
        }
        store.exhaustivity = .off

        let task = await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.sessions, [])

        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        await clock.advance(by: .seconds(2))
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.unfiled, [session])
        await task.cancel()
    }

    func testAnUnreachableDaemonSaysWhatToDoAndRetrying() async {
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.daemon.projects = { throw DaemonError.notRunning(socket: "/tmp/k.sock", reason: "refused") }
            $0.daemon.sessions = { _ in [] }
        }
        await store.send(.refresh)
        await store.receive(\.loadFailed) {
            $0.failure = DaemonError.notRunning(socket: "/tmp/k.sock", reason: "refused").description
        }

        store.dependencies.daemon.projects = { [] }
        await store.send(.refresh)
        await store.receive(\.loaded) {
            $0.failure = nil
        }
    }

    func testSelectingASessionOpensIt() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = ProjectsFeature.State()
        state.sessions = [session]
        let store = TestStore(initialState: state) { ProjectsFeature() }
        await store.send(.sessionSelected("s1")) {
            $0.selection = "s1"
        }
        await store.receive(\.delegate.openSession, session)
    }
}
