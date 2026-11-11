import ComposableArchitecture
import HumataDaemonClient
import HumataProjects
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
            \.destination.newProject.folderPicked, URL(filePath: "/code/humata", directoryHint: .isDirectory))
        XCTAssertEqual(store.state.destination?.newProject?.name, "humata")
        await store.send(\.destination.newProject.createTapped)
        await store.receive(\.destination.newProject.delegate.created)
        let project = try XCTUnwrap(store.state.projects.first)
        XCTAssertEqual(project.repositories.map(\.path), ["/code/humata"])
        XCTAssertEqual(store.state.destination?.newSession?.project, project)

        await store.send(\.destination.newSession.binding.backend, .scripted)
        await store.send(\.destination.newSession.binding.approvalMode, .allowEdits)
        await store.send(\.destination.newSession.startTapped)
        await store.receive(\.destination.newSession.delegate.started)
        let session = try XCTUnwrap(daemon.sessions.first)
        await store.receive(\.delegate.openSession, session)
        XCTAssertNil(store.state.destination)
        XCTAssertEqual(store.state.selection, .session(session.id))
        XCTAssertEqual(store.state.sessions(in: project.id), [session])
        XCTAssertNotEqual(session.workingDirectory, "/code/humata")
        XCTAssertEqual(session.approvalMode, .allowEdits)
        XCTAssertEqual(session.backend, .scripted)
    }

    func testANewChatWorksInTheFolderPickedOrOneOfItsOwn() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.newChatTapped)
        await store.send(\.destination.newSession.binding.backend, .scripted)
        await store.send(\.destination.newSession.binding.folder, "/notes")
        await store.send(\.destination.newSession.startTapped)
        await store.receive(\.destination.newSession.delegate.started)
        let inFolder = try XCTUnwrap(daemon.sessions.last)
        XCTAssertEqual(inFolder.workingDirectory, "/notes")
        XCTAssertNil(inFolder.projectID)
        XCTAssertEqual(store.state.unfiled, [inFolder])

        await store.send(.newChatTapped)
        await store.send(\.destination.newSession.binding.backend, .scripted)
        await store.send(\.destination.newSession.startTapped)
        await store.receive(\.destination.newSession.delegate.started)
        let own = try XCTUnwrap(daemon.sessions.last)
        XCTAssertNotEqual(own.id, inFolder.id)
        XCTAssertNotEqual(own.workingDirectory, inFolder.workingDirectory)
        XCTAssertNil(own.projectID)
    }

    func testAProjectNeedsANameAndAbsoluteFolders() {
        var state = NewProjectFeature.State()
        state.name = "humata"
        XCTAssertFalse(state.canCreate)
        state.repositories = [RepositoryRow(id: 0, path: "code/humata")]
        XCTAssertFalse(state.canCreate)
        state.repositories = [RepositoryRow(id: 0, path: "/code/humata"), RepositoryRow(id: 1)]
        XCTAssertTrue(state.canCreate)
        state.repositories.append(RepositoryRow(id: 2, path: "api"))
        XCTAssertFalse(state.canCreate)
        state.repositories.remove(id: 2)
        state.name = "  "
        XCTAssertFalse(state.canCreate)
    }

    func testAProjectOfSeveralRepositoriesWorksInAll() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: NewProjectFeature.State()) {
            NewProjectFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.folderPicked(URL(filePath: "/code/app", directoryHint: .isDirectory)))
        await store.send(.folderPicked(URL(filePath: "/code/api", directoryHint: .isDirectory)))
        await store.send(.folderPicked(URL(filePath: "/code/api", directoryHint: .isDirectory)))
        await store.send(.addRepositoryTapped)
        XCTAssertEqual(store.state.name, "app")
        await store.send(.createTapped)
        await store.receive(\.delegate.created)
        let project = try await daemon.client.projects().first
        XCTAssertEqual(project?.repositories.map(\.path), ["/code/app", "/code/api"])
        XCTAssertEqual(project?.repositories.map(\.name), ["app", "api"])
    }

    func testARefusedProjectSaysWhyAndCanBeTriedAgain() async {
        let refusal = DaemonError.refused(.invalidRequest, message: "There is no folder /code/gone.")
        var state = NewProjectFeature.State()
        state.name = "gone"
        state.repositories = [RepositoryRow(id: 0, path: "/code/gone")]
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
            $0.notifications = .previewValue
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
            $0.daemon.inbox = { [] }
        }
        await store.send(.refresh)
        await store.receive(\.loadFailed) {
            $0.failure = DaemonError.notRunning(socket: "/tmp/k.sock", reason: "refused").description
        }

        store.dependencies.daemon.projects = { [] }
        await store.send(.refresh)
        await store.receive(\.loaded) {
            $0.failure = nil
            $0.loadedOnce = true
        }
    }

    func testForksShowUnderTheSessionTheyCameFrom() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let root = try await daemon.client.start(StartRequest(backend: .scripted, projectID: project.id))
        let fork = try await daemon.client.fork(root.id, nil, "fork", false)
        let deeper = try await daemon.client.fork(fork.id, nil, "deeper", true)
        let other = try await daemon.client.start(StartRequest(backend: .scripted, projectID: project.id))
        var state = ProjectsFeature.State()
        state.projects = [project]
        state.sessions = IdentifiedArray(uniqueElements: try await daemon.client.sessions(nil))

        let tree = state.tree(in: project.id)
        XCTAssertEqual(tree.map(\.id), [root.id, other.id])
        XCTAssertEqual(tree.first?.children?.map(\.id), [fork.id])
        XCTAssertEqual(tree.first?.children?.first?.children?.map(\.id), [deeper.id])
        XCTAssertNil(tree.last?.children)
    }

    func testAnArchivedSessionLeavesTheTreeAndItsForksMoveUp() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let root = try await daemon.client.start(StartRequest(backend: .scripted, projectID: project.id))
        let fork = try await daemon.client.fork(root.id, nil, "fork", false)
        var state = ProjectsFeature.State()
        state.projects = [project]
        state.sessions = IdentifiedArray(uniqueElements: try await daemon.client.sessions(nil))
        let store = TestStore(initialState: state) {
            ProjectsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.archiveTapped(root.id, archived: true))
        await store.receive(\.updated)
        XCTAssertEqual(store.state.tree(in: project.id).map(\.id), [fork.id])
        XCTAssertEqual(store.state.archived.map(\.id), [root.id])

        await store.send(.archiveTapped(root.id, archived: false))
        await store.receive(\.updated)
        XCTAssertEqual(store.state.tree(in: project.id).map(\.id), [root.id])
        XCTAssertEqual(store.state.archived, [])
    }

    func testARefusedArchiveSaysWhy() async throws {
        let daemon = InMemoryDaemon { _ in [.run(command: "make", output: "")] }
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "build it", .enqueue)
        var state = ProjectsFeature.State()
        state.sessions = [session]
        let store = TestStore(initialState: state) {
            ProjectsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.archiveTapped(session.id, archived: true))
        await store.receive(\.updateFailed)
        XCTAssertNotNil(store.state.destination?.alert)
        XCTAssertEqual(store.state.archived, [])
    }

    func testRenamingASessionShowsTheNewName() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", name: "first"))
        var state = ProjectsFeature.State()
        state.sessions = [session]
        let store = TestStore(initialState: state) {
            ProjectsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.renameTapped(session.id))
        XCTAssertEqual(store.state.destination?.rename?.name, "first")
        await store.send(\.destination.rename.binding.name, "  release notes ")
        await store.send(\.destination.rename.saveTapped)
        await store.receive(\.destination.rename.delegate.renamed)
        XCTAssertNil(store.state.destination)
        XCTAssertEqual(store.state.sessions[id: session.id]?.name, "release notes")
        XCTAssertEqual(daemon.sessions.first?.name, "release notes")
    }

    func testComparingTwoSessionsShowsEachOnesChangesAndKeepsOne() async throws {
        let daemon = InMemoryDaemon { text in [.write(path: "/\(text).md", contents: text)] }
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let root = try await daemon.client.start(
            StartRequest(backend: .scripted, approvalMode: .allowEverything, projectID: project.id))
        let fork = try await daemon.client.fork(root.id, nil, "fork", false)
        try await daemon.client.send(root.id, "one", .enqueue)
        try await daemon.client.send(fork.id, "two", .enqueue)
        var state = ProjectsFeature.State()
        state.projects = [project]
        state.sessions = IdentifiedArray(uniqueElements: try await daemon.client.sessions(nil))
        XCTAssertEqual(state.comparable(to: root).map(\.id), [fork.id])
        let store = TestStore(initialState: state) {
            ProjectsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.compareTapped(root.id, with: fork.id))
        await store.send(\.destination.compare.task)
        await store.receive(\.destination.compare.loaded)
        let compare = try XCTUnwrap(store.state.destination?.compare)
        XCTAssertTrue(compare.leftChanges?.contains { $0.patch.contains("+one") } == true)
        XCTAssertTrue(compare.rightChanges?.contains { $0.patch.contains("+two") } == true)
        XCTAssertFalse(compare.rightChanges?.contains { $0.patch.contains("+one") } == true)

        await store.send(\.destination.compare.keepTapped, fork.id)
        await store.receive(\.destination.compare.marked)
        await store.receive(\.destination.compare.delegate.updated)
        XCTAssertEqual(store.state.sessions[id: fork.id]?.keeper, true)

        await store.send(\.destination.compare.keepTapped, root.id)
        await store.receive(\.destination.compare.marked)
        XCTAssertEqual(store.state.destination?.compare?.left.keeper, true)
        XCTAssertEqual(store.state.destination?.compare?.right.keeper, false)
        XCTAssertEqual(daemon.sessions.filter(\.keeper).map(\.id), [root.id])
    }

    func testSelectingASessionOpensIt() async {
        let session = Session(id: "s1", name: "first", backend: .scripted, workingDirectory: "/work")
        var state = ProjectsFeature.State()
        state.sessions = [session]
        let store = TestStore(initialState: state) { ProjectsFeature() }
        await store.send(.selected(.session("s1"))) {
            $0.selection = .session("s1")
        }
        await store.receive(\.delegate.openSession, session)
    }

    func testASessionThatComesToWaitIsNotifiedAndClickingItOpensIt() async throws {
        let daemon = InMemoryDaemon()
        let clock = TestClock()
        let posted = LockIsolated<[Notice]>([])
        let notified = expectation(description: "a notification posted")
        let (clicks, click) = AsyncStream.makeStream(of: Session.ID.self)
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.continuousClock = clock
            $0.notifications = NotificationClient(
                authorize: { true },
                post: { notice in
                    posted.withValue { $0.append(notice) }
                    notified.fulfill()
                },
                clicks: { clicks })
        }
        store.exhaustivity = .off

        // Waiting already when the app opens: in the inbox, not news.
        let old = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/old"))
        try await daemon.client.send(old.id, "hi", .enqueue)
        let task = await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(posted.value, [])

        let open = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/open"))
        let other = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/other", name: "Fix the login"))
        await clock.advance(by: .seconds(2))
        await store.receive(\.loaded)
        await store.send(.selected(.session(open.id)))
        try await daemon.client.send(open.id, "hi", .enqueue)
        try await daemon.client.send(other.id, "hi", .enqueue)
        await clock.advance(by: .seconds(2))
        await store.receive(\.loaded)
        await fulfillment(of: [notified], timeout: 5)
        XCTAssertEqual(posted.value.map(\.sessionID), [other.id])
        XCTAssertEqual(posted.value.first?.title, "Fix the login")

        // The same wait isn't news again.
        await clock.advance(by: .seconds(2))
        await store.receive(\.loaded)
        XCTAssertEqual(posted.value.count, 1)

        click.yield(other.id)
        await store.receive(\.notificationClicked)
        await store.receive(\.delegate.openSession)
        XCTAssertEqual(store.state.selection, .session(other.id))
        click.finish()
        await task.cancel()
    }

    func testANewSessionCanRunOnAHost() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let store = TestStore(initialState: NewSessionFeature.State(project: project)) {
            NewSessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.hostsLoaded)
        XCTAssertEqual(store.state.hosts.map(\.name), ["far"])
        await store.send(\.binding.host, "far")
        await store.send(.startTapped)
        await store.receive(\.delegate.started)
        XCTAssertEqual(daemon.sessions.first?.host, "far")
    }

    func testASessionOnAnUnreachableHostShowsOffline() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/code", host: "far"))
        daemon.setOffline("far", true)
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.sessions[id: session.id]?.hostOffline, true)
    }
}
