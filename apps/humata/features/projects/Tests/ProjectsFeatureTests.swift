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
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
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

    func testAutoApprovalsGoWhenTheAgentPickedHasNoJudge() async throws {
        let store = TestStore(initialState: NewSessionFeature.State(project: nil)) {
            NewSessionFeature()
        } withDependencies: {
            $0.daemon = InMemoryDaemon().client
        }
        store.exhaustivity = .off
        await store.send(\.binding.backend, .claude)
        await store.send(\.binding.approvalMode, .auto)
        await store.send(\.binding.backend, .copilot)
        XCTAssertEqual(store.state.approvalMode, .auto)
        await store.send(\.binding.backend, .local)
        XCTAssertEqual(store.state.approvalMode, .askEveryTime)
    }

    func testANewSessionStartsFromTheProjectsDefaultsOverEveryProjects() async throws {
        let daemon = InMemoryDaemon()
        daemon.setModels(
            [ModelChoice(id: "opus", name: "Opus", efforts: ["low", "high"], defaultEffort: "high")], for: .claude)
        _ = try await daemon.client.setSessionDefaults(
            nil, SessionDefaults(backend: .claude, model: "opus", effort: "low"))
        var project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let updated = try await daemon.client.setSessionDefaults(project.id, SessionDefaults(effort: "high"))
        project = try XCTUnwrap(updated)
        let store = TestStore(initialState: NewSessionFeature.State(project: project)) {
            NewSessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.defaultsLoaded)
        XCTAssertEqual(store.state.backend, .claude)
        XCTAssertEqual(store.state.model, "opus")
        XCTAssertEqual(store.state.effort, "high")

        await store.send(.startTapped)
        await store.receive(\.delegate.started)
        XCTAssertEqual(daemon.sessions.first?.model, "opus")
        XCTAssertEqual(daemon.sessions.first?.effort, "high")
    }

    func testWhatThePersonPicksBeforeTheDefaultsArriveStays() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.setSessionDefaults(nil, SessionDefaults(backend: .claude, model: "opus"))
        let store = TestStore(initialState: NewSessionFeature.State(project: nil)) {
            NewSessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.binding.backend, .copilot)
        await store.send(.defaultsLoaded(SessionDefaults(backend: .claude, model: "opus")))
        XCTAssertEqual(store.state.backend, .copilot)
        XCTAssertEqual(store.state.model, "")
    }

    func testAnEffortTheNewModelDoesntTakeGoesBackToItsDefault() async throws {
        let daemon = InMemoryDaemon()
        daemon.setModels(
            [
                ModelChoice(id: "opus", name: "Opus", efforts: ["low", "max"]),
                ModelChoice(id: "haiku", name: "Haiku"),
            ], for: .claude)
        let store = TestStore(initialState: NewSessionFeature.State(project: nil)) {
            NewSessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.binding.backend, .claude)
        await store.receive(\.modelsLoaded)
        await store.send(\.binding.model, "opus")
        await store.send(\.binding.effort, "max")
        await store.send(\.binding.model, "haiku")
        XCTAssertEqual(store.state.effort, "")
    }

    func testAClaudeModelTypedOrLeftToTheDefaultTakesAnEffort() async throws {
        let daemon = InMemoryDaemon()
        daemon.setModels(
            [
                ModelChoice(id: "opus", name: "Opus", efforts: ["low", "max"]),
                ModelChoice(id: "haiku", name: "Haiku"),
            ], for: .claude)
        let store = TestStore(initialState: NewSessionFeature.State(project: nil)) {
            NewSessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.binding.backend, .claude)
        await store.receive(\.modelsLoaded)
        XCTAssertEqual(store.state.effortModel?.efforts, ["low", "max"])
        await store.send(\.binding.effort, "max")
        await store.send(\.binding.model, "claude-opus-5-5")
        XCTAssertEqual(store.state.effort, "max")
        XCTAssertEqual(store.state.request.effort, "max")
        await store.send(\.binding.model, "claude-haiku-5-5")
        XCTAssertNil(store.state.effortModel)
        XCTAssertEqual(store.state.effort, "")
    }

    func testANewChatWorksInTheFolderPickedOrOneOfItsOwn() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
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

    func testAProjectCanBeAddedByTheURLsOfItsRepositories() async throws {
        let daemon = InMemoryDaemon()
        var state = NewProjectFeature.State()
        state.name = "app"
        state.repositories = [
            RepositoryRow(id: 0, path: "https://github.com/owner/app.git"),
            RepositoryRow(id: 1, path: "git@github.com:owner/api.git"),
            RepositoryRow(id: 2, path: "/code/docs"),
        ]
        XCTAssertTrue(state.canCreate)
        let store = TestStore(initialState: state) {
            NewProjectFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.createTapped)
        await store.receive(\.delegate.created)
        let projects = try await daemon.client.projects()
        let project = try XCTUnwrap(projects.first)
        XCTAssertEqual(project.repositories.map(\.name), ["app", "api", "docs"])
        XCTAssertEqual(
            project.repositories.map(\.url), ["https://github.com/owner/app.git", "git@github.com:owner/api.git", ""])
        XCTAssertTrue(project.repositories.allSatisfy { $0.path.hasPrefix("/") })
    }

    func testAProjectOfSeveralRepositoriesWorksInAll() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: NewProjectFeature.State()) {
            NewProjectFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
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
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
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
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
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

    /// A sidebar reading `daemon` on `clock`, with how many reads it made
    /// and how many it took in.
    private func polling(
        _ daemon: InMemoryDaemon, on clock: TestClock<Duration>
    ) -> (store: TestStoreOf<ProjectsFeature>, made: LockIsolated<Int>, taken: LockIsolated<Int>) {
        let made = LockIsolated(0)
        let taken = LockIsolated(0)
        var client = daemon.client
        let inbox = client.inbox
        client.inbox = {
            made.withValue { $0 += 1 }
            return try await inbox()
        }
        let counted = CombineReducers {
            Reduce<ProjectsFeature.State, ProjectsFeature.Action> { _, action in
                if case .loaded = action { taken.withValue { $0 += 1 } }
                return .none
            }
            ProjectsFeature()
        }
        let store = TestStore(initialState: ProjectsFeature.State()) {
            counted
        } withDependencies: { [client] in
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = client
            $0.continuousClock = clock
            $0.notifications = .previewValue
        }
        store.exhaustivity = .off
        return (store, made, taken)
    }

    func testAReadWithNothingNewIsNotTakenIn() async throws {
        let daemon = InMemoryDaemon()
        let clock = TestClock()
        let (store, made, taken) = polling(daemon, on: clock)
        _ = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))

        let task = await store.send(.task)
        await store.receive(\.loaded)
        // The reads are made in turn: by the third, the second, of the
        // same list as the first, is over.
        while made.value < 3 { await clock.advance(by: .seconds(2)) }
        let added = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/more"))
        await clock.advance(by: .seconds(2))
        await store.receive(\.loaded)
        XCTAssertNotNil(store.state.sessions[id: added.id])
        XCTAssertEqual(taken.value, 2)
        await task.cancel()
    }

    func testAChangeMadeHereThatTheDaemonDidNotKeepIsPutRightInTime() async throws {
        let daemon = InMemoryDaemon()
        let clock = TestClock()
        let (store, made, _) = polling(daemon, on: clock)
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))

        let task = await store.send(.task)
        await store.receive(\.loaded)
        // The sidebar loses a session the daemon still has, and every read
        // shows what the last did.
        await store.send(.loaded(projects: [], sessions: [], inbox: []))
        XCTAssertEqual(store.state.sessions, [])
        while made.value < 12 { await clock.advance(by: .seconds(2)) }
        await store.receive(\.loaded)
        XCTAssertNotNil(store.state.sessions[id: session.id])
        await task.cancel()
    }

    func testAnUnreachableDaemonSaysWhatToDoAndRetrying() async {
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon.projects = { throw DaemonError.notRunning(socket: "/tmp/k.sock", reason: "refused") }
            $0.daemon.sessions = { _ in [] }
            $0.daemon.inbox = { Inbox() }
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
            $0.badged = 0
            $0.now = Date(timeIntervalSince1970: 1_794_000_600)
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
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
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

    func testAnArchivedSessionIsDeletedOnlyOnceItsForksAreGone() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let root = try await daemon.client.start(StartRequest(backend: .scripted, projectID: project.id))
        let fork = try await daemon.client.fork(root.id, nil, "fork", false)
        for id in [root.id, fork.id] { _ = try await daemon.client.update(id, SessionUpdate(archived: true)) }
        var state = ProjectsFeature.State()
        state.projects = [project]
        state.sessions = IdentifiedArray(uniqueElements: try await daemon.client.sessions(nil))
        state.selection = .session(root.id)
        let store = TestStore(initialState: state) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.deleteSessionTapped(root.id))
        XCTAssertNotNil(store.state.destination?.alert)
        await store.send(.destination(.dismiss))

        await store.send(.deleteSessionTapped(fork.id))
        await store.send(.destination(.presented(.alert(.deleteSession(fork.id)))))
        await store.receive(\.sessionDeleted)
        await store.send(.deleteSessionTapped(root.id))
        await store.send(.destination(.presented(.alert(.deleteSession(root.id)))))
        await store.receive(\.sessionDeleted)

        XCTAssertEqual(store.state.archived, [])
        XCTAssertNil(store.state.selection)
        let left = try await daemon.client.sessions(nil)
        XCTAssertEqual(left, [])
    }

    func testASessionMovesToAHostAndBackOnlyBetweenTurns() async throws {
        let daemon = InMemoryDaemon { _ in [.run(command: "make", output: "")] }
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        var state = ProjectsFeature.State()
        state.sessions = [session]
        let store = TestStore(initialState: state) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.moveToHostTapped(session.id))
        await store.receive(\.moveHostsLoaded)
        XCTAssertNotNil(store.state.destination?.alert)
        await store.send(.destination(.presented(.alert(.moveSession(session.id, host: "far")))))
        await store.receive(\.sessionMoved)
        XCTAssertEqual(store.state.sessions[id: session.id]?.host, "far")

        await store.send(.moveToHostTapped(session.id))
        await store.receive(\.moveHostsLoaded)
        await store.send(.destination(.presented(.alert(.moveSession(session.id, host: "")))))
        await store.receive(\.sessionMoved)
        XCTAssertNil(store.state.sessions[id: session.id]?.host)

        // In a turn, it stays where it is and the person is told why.
        try await daemon.client.send(session.id, "build it", .enqueue)
        await store.send(.moveToHostTapped(session.id))
        await store.receive(\.moveHostsLoaded)
        await store.send(.destination(.presented(.alert(.moveSession(session.id, host: "far")))))
        await store.receive(\.sessionMoveFailed)
        XCTAssertNotNil(store.state.destination?.alert)
        let sessions = try await daemon.client.sessions(nil)
        XCTAssertNil(sessions.first { $0.id == session.id }?.host)
    }

    func testASessionIsBusyWhileTheHostsToMoveItToAreRead() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        var state = ProjectsFeature.State()
        state.sessions = [session]
        let client = daemon.client
        let reads = LockIsolated(0)
        // The hosts take a while to answer.
        let (answered, answer) = AsyncStream<Void>.makeStream()
        let store = TestStore(initialState: state) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = client
            $0.daemon.hosts = {
                reads.withValue { $0 += 1 }
                for await _ in answered { break }
                return try await client.hosts()
            }
        }
        store.exhaustivity = .off

        await store.send(.moveToHostTapped(session.id))
        XCTAssertEqual(store.state.busy, [session.id])
        // A second go while they're read asks nothing more.
        await store.send(.moveToHostTapped(session.id))
        await Task.megaYield()
        XCTAssertEqual(reads.value, 1)

        answer.yield()
        await store.receive(\.moveHostsLoaded)
        XCTAssertEqual(store.state.busy, [])
        XCTAssertNotNil(store.state.destination?.alert)
    }

    func testDeletingAnUnarchivedSessionIsRefused() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        var state = ProjectsFeature.State()
        state.sessions = [session]
        let store = TestStore(initialState: state) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.deleteSessionTapped(session.id))
        await store.send(.destination(.presented(.alert(.deleteSession(session.id)))))
        await store.receive(\.sessionDeleteFailed)
        XCTAssertNotNil(store.state.destination?.alert)
        XCTAssertEqual(store.state.sessions.map(\.id), [session.id])
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
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.archiveTapped(session.id, archived: true))
        await store.receive(\.updateFailed)
        XCTAssertNotNil(store.state.destination?.alert)
        XCTAssertEqual(store.state.archived, [])
    }

    func testStopAndArchiveEndsTheTurnThenArchives() async throws {
        let daemon = InMemoryDaemon { _ in [.run(command: "make", output: "")] }
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "build it", .enqueue)
        // What's queued goes too, rather than starting and holding the archive.
        try await daemon.client.send(session.id, "then test it", .enqueue)
        var state = ProjectsFeature.State()
        state.sessions = [session]
        let store = TestStore(initialState: state) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.stopAndArchiveTapped(session.id))
        XCTAssertTrue(store.state.busy.contains(session.id))
        await store.receive(\.updated)
        XCTAssertFalse(store.state.busy.contains(session.id))
        XCTAssertEqual(store.state.archived.map(\.id), [session.id])
        let history = try await daemon.client.history(session.id)
        XCTAssertTrue(history.contains { $0.kind == .turnEnded(aborted: true) })
    }

    func testASessionInATurnIsNeitherArchivedNorMoved() async throws {
        let daemon = InMemoryDaemon()
        var session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        session.state = .working
        var state = ProjectsFeature.State()
        state.sessions = [session]
        let updates = LockIsolated(0)
        let hostReads = LockIsolated(0)
        let client = daemon.client
        let store = TestStore(initialState: state) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.daemon.update = { id, update in
                updates.withValue { $0 += 1 }
                return try await client.update(id, update)
            }
            $0.daemon.hosts = {
                hostReads.withValue { $0 += 1 }
                return try await client.hosts()
            }
        }
        store.exhaustivity = .off

        await store.send(.archiveTapped(session.id, archived: true))
        await store.send(.moveToHostTapped(session.id))
        await Task.megaYield()
        XCTAssertEqual(updates.value, 0)
        XCTAssertEqual(hostReads.value, 0)
        XCTAssertEqual(store.state.busy, [])
        XCTAssertEqual(store.state.archived, [])
        XCTAssertNil(store.state.destination)
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
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
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
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.compareTapped(root.id, with: fork.id))
        await store.send(\.destination.compare.task)
        await store.receive(\.destination.compare.loaded)
        let compare = try XCTUnwrap(store.state.destination?.compare)
        let leftLines = compare.leftChanges?.patches.flatMap(\.lines) ?? []
        let rightLines = compare.rightChanges?.patches.flatMap(\.lines) ?? []
        XCTAssertTrue(leftLines.contains { $0.contains("+one") })
        XCTAssertTrue(rightLines.contains { $0.contains("+two") })
        XCTAssertFalse(rightLines.contains { $0.contains("+one") })

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
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
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

        // The same wait isn't news again, read now or once every read is
        // taken in again.
        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(posted.value.count, 1)

        click.yield(other.id)
        await store.receive(\.notificationClicked)
        await store.receive(\.delegate.openSession)
        XCTAssertEqual(store.state.selection, .session(other.id))
        click.finish()
        await task.cancel()
    }

    func testWhileHumataIsBehindTheOpenSessionIsNotifiedQuietlyOfAReply() async throws {
        let daemon = InMemoryDaemon()
        let clock = TestClock()
        let posted = LockIsolated<[Notice]>([])
        let badges = LockIsolated<[Int]>([])
        let notified = expectation(description: "a notification posted")
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.continuousClock = clock
            $0.notifications = NotificationClient(
                authorize: { true },
                post: { notice in
                    posted.withValue { $0.append(notice) }
                    notified.fulfill()
                },
                clicks: { AsyncStream { $0.finish() } })
            $0.appActivity = AppActivity(
                isActive: { false }, activations: { AsyncStream { $0.finish() } },
                setBadge: { count in badges.withValue { $0.append(count) } })
        }
        store.exhaustivity = .off

        let open = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/open"))
        let task = await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.selected(.session(open.id)))
        try await daemon.client.send(open.id, "hi", .enqueue)
        await clock.advance(by: .seconds(2))
        await store.receive(\.loaded)
        await fulfillment(of: [notified], timeout: 5)
        XCTAssertEqual(posted.value.map(\.sessionID), [open.id])
        XCTAssertEqual(posted.value.map(\.silent), [true])
        // A reply only needs looking over, so the Dock doesn't count it.
        XCTAssertEqual(badges.value, [0])
        await task.cancel()
    }

    func testTheDockCountsOnlyTheSessionsBlockedOnYou() async throws {
        let daemon = InMemoryDaemon { message in
            message == "build" ? [.run(command: "make", output: ""), .say("Built.")] : [.say("Done.")]
        }
        let badges = LockIsolated<[Int]>([])
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.notifications.post = { _ in }
            $0.appActivity = AppActivity(
                isActive: { true }, activations: { AsyncStream { $0.finish() } },
                setBadge: { count in badges.withValue { $0.append(count) } })
        }
        store.exhaustivity = .off

        let building = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/build"))
        try await daemon.client.send(building.id, "build", .enqueue)
        for name in ["a", "b", "c"] {
            let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/\(name)"))
            try await daemon.client.send(session.id, "hi", .enqueue)
        }
        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.inbox.count, 4)
        XCTAssertEqual(store.state.inbox.first?.reason, .approval)
        XCTAssertEqual(store.state.inbox.last?.session.lastMessage, "Done.")
        XCTAssertEqual(store.state.blockedCount, 1, "replies wait on nobody")
        await store.finish()
        XCTAssertEqual(badges.value, [1])
    }

    func testHumataOpensOnTheInboxWhenSomethingIsBlocked() async throws {
        let daemon = InMemoryDaemon { message in
            message == "build" ? [.run(command: "make", output: ""), .say("Built.")] : [.say("Done.")]
        }
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.notifications.post = { _ in }
        }
        store.exhaustivity = .off

        let building = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/build"))
        try await daemon.client.send(building.id, "build", .enqueue)
        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.selection, .inbox)

        // Picked later, another session stays picked.
        await store.send(.selected(.session(building.id)))
        await store.receive(\.delegate.openSession)
        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.selection, .session(building.id))
    }

    func testHumataOpensOnNothingWhenOnlyRepliesWait() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "hi", .enqueue)
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
        XCTAssertFalse(store.state.inbox.isEmpty)
        XCTAssertNil(store.state.selection)
    }

    func testAReplyIsMarkedAsReadFromTheInbox() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "hi", .enqueue)
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
        XCTAssertEqual(store.state.inbox.map(\.reason), [.reply])

        await store.send(.markReadTapped(session.id))
        await store.receive(\.loaded)
        XCTAssertTrue(store.state.inbox.isEmpty)
        let seen = try await daemon.client.sessions(nil).first
        XCTAssertEqual(seen?.seenTurns, seen?.turns)
    }

    func testAReplyThatCouldntBeSentIsKeptAndOneThatWasIsCleared() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        let first = daemon.addRequest(session.id, .question, "Email or phone?")
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
        // Answered elsewhere meanwhile, so this reply can't be sent.
        try await daemon.client.answerRequest(first.id, .dismiss)
        await store.send(.draftChanged(first.id, "Email."))
        await store.send(.requestAnswered(first.id, .reply("Email.")))
        await store.receive(\.answerFailed)
        XCTAssertEqual(store.state.drafts[first.id], "Email.")

        let second = daemon.addRequest(session.id, .question, "Which provider?")
        await store.send(.draftChanged(second.id, "Apple."))
        await store.send(.requestAnswered(second.id, .reply("Apple.")))
        await store.receive(\.requestAnswerSent)
        XCTAssertNil(store.state.drafts[second.id])
    }

    func testAnAnsweredRequestDoesntComeBackFromAReadBegunBeforeTheAnswer() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        let question = daemon.addRequest(session.id, .question, "Email or phone?")
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
        let stale = store.state.inbox
        XCTAssertEqual(stale.compactMap(\.request?.id), [question.id])
        await store.send(.requestAnswered(question.id, .reply("Email.")))
        await store.receive(\.requestAnswerSent)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.inbox.compactMap(\.request?.id), [])
        // The read begun before the answer arrives last.
        await store.send(.loaded(projects: [], sessions: [session], inbox: stale, approvals: []))
        XCTAssertEqual(store.state.inbox.compactMap(\.request?.id), [])
    }

    func testANewSessionCanRunOnAHost() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let store = TestStore(initialState: NewSessionFeature.State(project: project)) {
            NewSessionFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
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

    func testTheModelIsPickedFromWhatTheAgentOffersOrTypedWhenItCantList() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        var initial = NewSessionFeature.State(project: project)
        initial.backend = .claude
        let store = TestStore(initialState: initial) {
            NewSessionFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.continuousClock = ImmediateClock()
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.modelsLoaded)
        let claude = store.state.models.map(\.id)
        XCTAssertFalse(claude.isEmpty)

        daemon.setModels(nil, for: .copilot)
        await store.send(\.binding.backend, .copilot)
        await store.receive(\.modelsLoaded)
        XCTAssertEqual(store.state.models, [])
        XCTAssertNotNil(store.state.modelsUnavailable)

        await store.send(\.binding.backend, .claude)
        await store.receive(\.modelsLoaded)
        XCTAssertEqual(store.state.models.map(\.id), claude)
        XCTAssertNil(store.state.modelsUnavailable)

        await store.send(\.binding.backend, .copilot)
        await store.receive(\.modelsLoaded)
        await store.send(\.binding.model, "gpt-5-mini")
        await store.send(.startTapped)
        await store.receive(\.delegate.started)
        XCTAssertEqual(daemon.sessions.first?.model, "gpt-5-mini")
    }

    func testALocalModelSessionTakesAModelItsServerRunsNeverAnotherAgents() async throws {
        let daemon = InMemoryDaemon()
        daemon.setModels([ModelChoice(id: "apple", name: "Apple")], for: .local)
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        var initial = NewSessionFeature.State(project: project)
        initial.backend = .claude
        let store = TestStore(initialState: initial) {
            NewSessionFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.binding.model, "opus")
        await store.send(\.binding.backend, .local)
        await store.receive(\.modelsLoaded)
        XCTAssertEqual(store.state.model, "apple")

        daemon.setModels([], for: .local)
        await store.send(\.binding.backend, .claude)
        await store.receive(\.modelsLoaded)
        await store.send(\.binding.backend, .local)
        await store.receive(\.modelsLoaded)
        XCTAssertFalse(store.state.canStart)
        await store.send(.startTapped)
        XCTAssertTrue(daemon.sessions.isEmpty)
    }

    func testAnAgentKeepsATypedModelItsListLacks() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        var initial = NewSessionFeature.State(project: project)
        initial.backend = .claude
        let store = TestStore(initialState: initial) {
            NewSessionFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.binding.model, "claude-next")
        await store.send(.task)
        await store.receive(\.modelsLoaded)
        XCTAssertFalse(store.state.models.contains(where: { $0.id == "claude-next" }))
        await store.send(.startTapped)
        await store.receive(\.delegate.started)
        XCTAssertEqual(daemon.sessions.first?.model, "claude-next")
    }

    /// The approval mode carries over; the agent and model come from the
    /// defaults instead.
    func testTheNextSessionKeepsTheLastApprovalModeButNotItsAgent() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let store = TestStore(initialState: NewSessionFeature.State(project: project)) {
            NewSessionFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.binding.backend, .claude)
        await store.send(\.binding.model, "opus")
        await store.send(\.binding.approvalMode, .allowEdits)
        await store.send(.startTapped)
        await store.receive(\.delegate.started)

        let next = NewSessionFeature.State(project: nil)
        XCTAssertEqual(next.model, "")
        XCTAssertEqual(next.approvalMode, .allowEdits)
    }

    func testCopilotAndLocalModelsUseTheModelServerGiven() throws {
        var state = NewSessionFeature.State(project: nil)
        state.modelServer = " http://127.0.0.1:11434/v1 "
        state.backend = .copilot
        let copilot = try XCTUnwrap(state.backendConfig?.data(using: .utf8))
        let provider = try XCTUnwrap(
            (try JSONSerialization.jsonObject(with: copilot) as? [String: [String: String]])?["provider"])
        XCTAssertEqual(provider["base_url"], "http://127.0.0.1:11434/v1")
        state.backend = .local
        let local = try XCTUnwrap(state.backendConfig?.data(using: .utf8))
        XCTAssertEqual(
            (try JSONSerialization.jsonObject(with: local) as? [String: String])?["base_url"],
            "http://127.0.0.1:11434/v1")
        state.backend = .claude
        XCTAssertNil(state.backendConfig)
        state.backend = .copilot
        state.modelServer = ""
        XCTAssertNil(state.backendConfig)
    }

    func testARequestIsAllowedFromTheInbox() async throws {
        let daemon = InMemoryDaemon { _ in [.run(command: "make", output: ""), .say("Built.")] }
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "hi", .enqueue)
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
        let entry = try XCTUnwrap(store.state.inbox.first { $0.id == session.id })
        XCTAssertEqual(entry.reason, .approval)
        let request = try XCTUnwrap(entry.session.pending.first)

        await store.send(.inboxAnswered(session.id, request: request.requestID, approved: true))
        await store.receive(\.loaded)
        let history = try await daemon.client.history(session.id)
        XCTAssertTrue(
            history.contains {
                $0.kind == .permissionCompleted(requestID: request.requestID, approved: true, automatic: false)
            })
        XCTAssertFalse(
            store.state.inbox.contains { $0.id == session.id && $0.session.pending.contains(request) })
    }

    func testTheDockCountGoesDownWhenARequestIsAnsweredFromTheInbox() async throws {
        let daemon = InMemoryDaemon { _ in [.run(command: "make", output: ""), .say("Built.")] }
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "hi", .enqueue)
        let badges = LockIsolated<[Int]>([])
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.notifications.post = { _ in }
            $0.appActivity = AppActivity(
                isActive: { true }, activations: { AsyncStream { $0.finish() } },
                setBadge: { count in badges.withValue { $0.append(count) } })
        }
        store.exhaustivity = .off

        await store.send(.refresh)
        await store.receive(\.loaded)
        let request = try XCTUnwrap(store.state.inbox.first { $0.id == session.id }?.session.pending.first)
        await store.send(.inboxAnswered(session.id, request: request.requestID, approved: true))
        await store.receive(\.loaded)
        await store.finish()
        XCTAssertEqual(badges.value, [1, 0])
    }

    private func inboxStore(_ daemon: InMemoryDaemon) -> TestStoreOf<ProjectsFeature> {
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.notifications.post = { _ in }
        }
        store.exhaustivity = .off
        return store
    }

    func testAnAnsweredPermissionDoesntComeBackFromAReadBegunBeforeTheAnswer() async throws {
        let daemon = InMemoryDaemon { _ in [.run(command: "make", output: ""), .say("Built.")] }
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "hi", .enqueue)
        let answers = LockIsolated<[Bool]>([])
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            // The answer is still on its way when the older read comes in.
            $0.daemon.approve = { _, _, approved in
                answers.withValue { $0.append(approved) }
                try await Task.never()
            }
            $0.notifications.post = { _ in }
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.loaded)
        let stale = store.state.inbox
        let request = try XCTUnwrap(stale.first { $0.reason == .approval }?.session.pending.first)

        let first = await store.send(.inboxAnswered(session.id, request: request.requestID, approved: true))
        XCTAssertFalse(store.state.inbox.contains { $0.reason == .approval })
        // A second click sends no second answer.
        await store.send(.inboxAnswered(session.id, request: request.requestID, approved: false))
        await store.send(.loaded(projects: [], sessions: [session], inbox: stale, approvals: []))
        XCTAssertFalse(store.state.inbox.contains { $0.reason == .approval })
        await Task.megaYield()
        XCTAssertEqual(answers.value, [true])
        await first.cancel()
    }

    func testAPermissionWhoseAnswerFailedComesBack() async throws {
        let daemon = InMemoryDaemon { _ in [.run(command: "make", output: ""), .say("Built.")] }
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "hi", .enqueue)
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.daemon.approve = { _, _, _ in throw DaemonError.disconnected }
            $0.notifications.post = { _ in }
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.loaded)
        let request = try XCTUnwrap(store.state.inbox.first { $0.reason == .approval }?.session.pending.first)

        await store.send(.inboxAnswered(session.id, request: request.requestID, approved: true))
        await store.receive(\.answerFailed)
        await store.receive(\.loaded)
        XCTAssertNotNil(store.state.destination?.alert)
        XCTAssertTrue(store.state.inbox.contains { $0.session.pending.contains(request) })
    }

    func testASessionBeingArchivedTakesNoSecondArchive() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        var state = ProjectsFeature.State()
        state.sessions = [session]
        let updates = LockIsolated(0)
        let store = TestStore(initialState: state) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            // The first archive is still going when the second tap comes.
            $0.daemon.update = { _, _ in
                updates.withValue { $0 += 1 }
                return try await Task.never()
            }
        }
        store.exhaustivity = .off

        let first = await store.send(.archiveTapped(session.id, archived: true))
        XCTAssertEqual(store.state.busy, [session.id])
        await store.send(.archiveTapped(session.id, archived: true))
        await Task.megaYield()
        XCTAssertEqual(updates.value, 1)
        await first.cancel()
    }

    func testTryingAgainFromTheInboxTwiceSendsOnce() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "build it", .enqueue)
        let sent = LockIsolated<[String]>([])
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            // The first retry is still being sent when the second tap comes.
            $0.daemon.send = { _, text, _ in
                sent.withValue { $0.append(text) }
                return try await Task.never()
            }
            $0.notifications.post = { _ in }
        }
        store.exhaustivity = .off

        let first = await store.send(.inboxRetryTapped(session.id))
        XCTAssertEqual(store.state.retrying, [session.id])
        await store.send(.inboxRetryTapped(session.id))
        await Task.megaYield()
        XCTAssertEqual(sent.value, ["build it"])
        await first.cancel()
    }

    func testAllowingEverythingFromTheInboxLetsTheWaitingCommandRun() async throws {
        let daemon = InMemoryDaemon { _ in [.run(command: "make", output: ""), .say("Built.")] }
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "hi", .enqueue)
        let store = inboxStore(daemon)
        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.inbox.first?.reason, .approval)

        await store.send(.inboxModeWidened(session.id, .allowEverything))
        await store.receive(\.loaded)
        XCTAssertFalse(store.state.inbox.contains { $0.reason == .approval })
        let sessions = try await daemon.client.sessions(nil)
        XCTAssertEqual(sessions.first?.approvalMode, .allowEverything)
    }

    func testTryingAgainFromTheInboxSendsTheLastMessageAgain() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "first", .enqueue)
        try await daemon.client.send(session.id, "build it", .enqueue)
        let store = inboxStore(daemon)

        await store.send(.inboxRetryTapped(session.id))
        await store.receive(\.loaded)
        let sent = try await daemon.client.history(session.id).compactMap { event -> String? in
            if case .userMessage(let text, _) = event.kind { text } else { nil }
        }
        XCTAssertEqual(sent, ["first", "build it", "build it"])
    }

    func testNextWaitingGoesThroughTheBlockedSessionsAndWraps() async throws {
        let daemon = InMemoryDaemon { _ in [.run(command: "make", output: ""), .say("Built.")] }
        for name in ["a", "b"] {
            let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/\(name)"))
            try await daemon.client.send(session.id, "hi", .enqueue)
        }
        let store = inboxStore(daemon)
        await store.send(.refresh)
        await store.receive(\.loaded)

        var visited: [SidebarItem?] = []
        for _ in 0..<3 {
            await store.send(.nextWaitingTapped)
            await store.receive(\.selected)
            visited.append(store.state.selection)
        }
        XCTAssertNotEqual(visited[0], visited[1])
        XCTAssertEqual(visited[0], visited[2])
    }

    func testNextWaitingWithNothingBlockedOpensTheInbox() async {
        let store = inboxStore(InMemoryDaemon())
        await store.send(.nextWaitingTapped)
        await store.receive(\.selected)
        XCTAssertEqual(store.state.selection, .inbox)
    }

    func testAnAgentsQuestionIsRepliedToAndItsMoveApprovedFromTheInbox() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, projectID: project.id, itemID: task.id))
        let question = daemon.addRequest(session.id, .question, "Email or phone?")
        let move = daemon.addRequest(session.id, .move(item: task.id, status: .done), "Login works.")
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
        XCTAssertEqual(store.state.inbox.map(\.reason), [.question, .move])

        await store.send(.requestAnswered(question.id, .reply("Email.")))
        await store.receive(\.loaded)
        let history = try await daemon.client.history(session.id)
        XCTAssertTrue(history.contains { $0.kind == .userMessage("Email.") })

        // Moving it to Done asks about its session, which is stopped and
        // archived once the move is made.
        await store.send(.requestAnswered(move.id, .approve))
        XCTAssertNotNil(store.state.destination?.alert)
        await store.send(.destination(.presented(.alert(.closeItem(move.id, .approve, archive: true)))))
        await store.receive(\.loaded)
        let status = try await daemon.client.items(project.id).first?.status
        XCTAssertEqual(status, .done)
        XCTAssertFalse(store.state.inbox.contains { $0.request != nil })
        let archived = try await daemon.client.sessions(nil).first { $0.id == session.id }?.archived
        XCTAssertEqual(archived, true)

        // Answered here already: pressed again, it is not answered again.
        await store.send(.requestAnswered(move.id, .dismiss))
        XCTAssertNil(store.state.destination)
    }

    func testAMoveCanBeMadeToAnotherColumnThanAsked() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, projectID: project.id, itemID: task.id))
        let move = daemon.addRequest(session.id, .move(item: task.id, status: .done), "Login works.")
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
        await store.send(.requestAnswered(move.id, .move(.inReview)))
        await store.receive(\.loaded)
        let status = try await daemon.client.items(project.id).first?.status
        XCTAssertEqual(status, .inReview)
        XCTAssertTrue(store.state.inbox.isEmpty)
    }

    func testASessionMovesToATaskAndBackToTheProject() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, name: "first", projectID: project.id))
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.refresh)
        await store.receive(\.loaded)
        await store.send(.moveTapped(session.id))
        await store.send(\.destination.move.task)
        await store.receive(\.destination.move.itemsLoaded)
        XCTAssertEqual(store.state.destination?.move?.items?.map(\.id), [task.id])
        await store.send(\.destination.move.binding.itemID, task.id)
        await store.send(\.destination.move.moveTapped)
        await store.receive(\.destination.move.delegate.moved)
        XCTAssertNil(store.state.destination)
        XCTAssertEqual(store.state.sessions[id: session.id]?.itemID, task.id)

        await store.send(.moveTapped(session.id))
        await store.send(\.destination.move.binding.itemID, nil)
        await store.send(\.destination.move.moveTapped)
        await store.receive(\.destination.move.delegate.moved)
        XCTAssertNil(store.state.sessions[id: session.id]?.itemID)
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
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.sessions[id: session.id]?.hostOffline, true)
    }

    private static func at(_ minutes: Double) -> Date {
        Date(timeIntervalSince1970: 1_794_000_000 + minutes * 60)
    }

    func testProjectsAndSessionsComeMostRecentlyActiveFirst() {
        let old = Project(id: "p1", name: "old", repositories: [], created: Self.at(0))
        let busy = Project(id: "p2", name: "busy", repositories: [], created: Self.at(0))
        let empty = Project(id: "p3", name: "empty", repositories: [], created: Self.at(0))
        var state = ProjectsFeature.State()
        state.projects = [old, busy, empty]
        state.sessions = [
            Session(
                id: "a", name: "a", backend: .scripted, workingDirectory: "/", projectID: "p1", lastActive: Self.at(1)),
            Session(
                id: "b", name: "b", backend: .scripted, workingDirectory: "/", projectID: "p2", lastActive: Self.at(2)),
            Session(
                id: "c", name: "c", backend: .scripted, workingDirectory: "/", parentID: "b", projectID: "p2",
                lastActive: Self.at(9)),
            Session(
                id: "d", name: "d", backend: .scripted, workingDirectory: "/", projectID: "p2", lastActive: Self.at(5)),
            Session(id: "e", name: "e", backend: .scripted, workingDirectory: "/", projectID: "p2"),
        ]

        // p2's session e has done nothing yet, so p2 is as new as the empty
        // p3, and stays before it.
        XCTAssertEqual(state.sortedProjects.map(\.id), ["p2", "p3", "p1"])
        // e has done nothing yet, so it was just started; b's fork c is the
        // latest of the rest.
        XCTAssertEqual(state.tree(in: "p2").map(\.id), ["e", "b", "d"])
    }

    func testTheSidebarHoldsStillWhileAReplyStreamsAndMovesWhenATurnEnds() async {
        let project = Project(id: "p1", name: "Humata", repositories: [], created: Self.at(0))
        func session(_ id: String, _ state: SessionState, turns: Int, at minutes: Double) -> Session {
            Session(
                id: id, name: id, backend: .scripted, workingDirectory: "/", state: state, projectID: "p1",
                turns: turns, lastActive: Self.at(minutes))
        }
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Self.at(20))
            $0.notifications = .previewValue
        }
        store.exhaustivity = .off
        func load(_ sessions: [Session]) async {
            await store.send(.loaded(projects: [project], sessions: sessions, inbox: []))
        }

        await load([session("a", .working, turns: 0, at: 2), session("b", .idle, turns: 1, at: 5)])
        XCTAssertEqual(store.state.tree(in: "p1").map(\.id), ["b", "a"])

        // a streams its reply: it was last active later, but stays put.
        await load([session("a", .working, turns: 0, at: 8), session("b", .idle, turns: 1, at: 5)])
        XCTAssertEqual(store.state.tree(in: "p1").map(\.id), ["b", "a"])

        // Its turn ends, so it moves up.
        await load([session("a", .idle, turns: 1, at: 9), session("b", .idle, turns: 1, at: 5)])
        XCTAssertEqual(store.state.tree(in: "p1").map(\.id), ["a", "b"])

        // A new session comes in at the top.
        await load([
            session("a", .idle, turns: 1, at: 9), session("b", .idle, turns: 1, at: 5),
            session("c", .idle, turns: 0, at: 10),
        ])
        XCTAssertEqual(store.state.tree(in: "p1").map(\.id), ["c", "a", "b"])
    }

    func testAReadWhileASessionWorksLeavesWhatTheSidebarShowsAsItWas() async {
        let project = Project(id: "p1", name: "Humata", repositories: [], created: Self.at(0))
        // What a read brings `seconds` into the same minute: the session at
        // work is active again and has said more, and its earlier reply
        // still waits in the inbox.
        func read(seconds: Double, said: String) -> (sessions: [Session], inbox: [InboxEntry]) {
            var working = Session(
                id: "a", name: "a", backend: .scripted, workingDirectory: "/", state: .working, projectID: "p1",
                lastActive: Self.at(8).addingTimeInterval(seconds))
            var waiting = working
            working.lastMessage = said
            waiting.lastMessage = "Done."
            return ([working], [InboxEntry(session: waiting, reason: .reply)])
        }
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Self.at(20))
            $0.notifications = .previewValue
        }
        store.exhaustivity = .off
        let first = read(seconds: 1, said: "One.")
        await store.send(.loaded(projects: [project], sessions: first.sessions, inbox: first.inbox))
        await store.finish()

        // Nothing changes, so nothing showing the sidebar's state is drawn
        // again.
        store.exhaustivity = .on
        let next = read(seconds: 3, said: "One. Two.")
        await store.send(.loaded(projects: [project], sessions: next.sessions, inbox: next.inbox))
    }

    func testSearchFindsProjectsAndSessionsByName() {
        var state = ProjectsFeature.State()
        state.projects = [
            Project(id: "p1", name: "Humata", repositories: [], created: Self.at(0)),
            Project(id: "p2", name: "Notes", repositories: [], created: Self.at(0)),
        ]
        state.sessions = [
            Session(id: "a", name: "Release notes", backend: .scripted, workingDirectory: "/", projectID: "p1"),
            Session(id: "b", name: "Fix login", backend: .scripted, workingDirectory: "/", archived: true),
        ]
        XCTAssertNil(state.matches)

        state.search = "notes"
        XCTAssertEqual(state.matches?.projects.map(\.id), ["p2"])
        XCTAssertEqual(state.matches?.sessions.map(\.id), ["a"])

        state.search = "LOGIN"
        XCTAssertEqual(state.matches?.projects.map(\.id), [])
        XCTAssertEqual(state.matches?.sessions.map(\.id), ["b"])
    }

    func testAProjectWithSessionsWorkingInItIsNotDeleted() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let session = try await daemon.client.start(StartRequest(backend: .scripted, projectID: project.id))
        var state = ProjectsFeature.State()
        state.projects = [project]
        state.sessions = [session]
        let store = TestStore(initialState: state) {
            ProjectsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.deleteProjectTapped(project.id))
        XCTAssertNotNil(store.state.destination?.alert)
        do {
            try await daemon.client.deleteProject(project.id)
            XCTFail("a project with a session working in it was deleted")
        } catch DaemonError.refused(let code, _) {
            XCTAssertEqual(code, .busy)
        }
        let projects = try await daemon.client.projects()
        XCTAssertEqual(projects.map(\.id), [project.id])
    }

    func testADeletedProjectKeepsItsArchivedSessions() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        var session = try await daemon.client.start(StartRequest(backend: .scripted, projectID: project.id))
        session = try await daemon.client.update(session.id, SessionUpdate(archived: true))
        var state = ProjectsFeature.State()
        state.projects = [project]
        state.sessions = [session]
        state.selection = .board(project.id)
        let store = TestStore(initialState: state) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Self.at(10))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.deleteProjectTapped(project.id))
        await store.send(.destination(.presented(.alert(.deleteProject(project.id)))))
        await store.receive(\.projectDeleted)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.projects, [])
        XCTAssertNil(store.state.selection)
        XCTAssertEqual(store.state.archived.map(\.id), [session.id])
        XCTAssertNil(store.state.sessions[id: session.id]?.projectID)
        let projects = try await daemon.client.projects()
        XCTAssertEqual(projects, [])
    }

    private func importable(_ id: String, _ summary: String, minutesAgo: Double) -> ImportableSession {
        ImportableSession(
            backend: .claude, id: id, workingDirectory: "/code/\(id)", summary: summary,
            updated: Date(timeIntervalSince1970: 1_794_000_600 - minutesAgo * 60))
    }

    func testAnImportedSessionOpensWithItsHistoryInTheProjectPicked() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        daemon.addImportable(importable("old", "rename the module", minutesAgo: 90))
        daemon.addImportable(importable("new", "fix the login", minutesAgo: 5))
        var initial = ProjectsFeature.State()
        initial.projects = [project]
        let store = TestStore(initialState: initial) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.importTapped)
        await store.send(\.destination.importSession.task)
        await store.receive(\.destination.importSession.loaded)
        XCTAssertEqual(store.state.destination?.importSession?.sessions?.map(\.id), ["new", "old"])
        await store.send(\.destination.importSession.binding.projectID, project.id)
        await store.send(\.destination.importSession.binding.selection, "new")
        await store.send(\.destination.importSession.importTapped)
        await store.receive(\.destination.importSession.delegate.imported)
        let session = try XCTUnwrap(daemon.sessions.first)
        await store.receive(\.delegate.openSession, session)
        XCTAssertNil(store.state.destination)
        XCTAssertEqual(store.state.selection, .session(session.id))
        XCTAssertEqual(session.workingDirectory, "/code/new")
        XCTAssertEqual(session.projectID, project.id)
        XCTAssertEqual(session.seenTurns, session.turns, "what happened outside Humata isn't unread")
        XCTAssertEqual(
            daemon.history(session.id).compactMap {
                if case .userMessage(let text, _) = $0.kind { text } else { nil }
            },
            ["fix the login"])

        await store.send(.importTapped)
        await store.send(\.destination.importSession.task)
        await store.receive(\.destination.importSession.loaded)
        XCTAssertEqual(store.state.destination?.importSession?.sessions?.map(\.id), ["old"])
    }

    func testPickingAHostListsItsSessionsAndImportingOneTwiceIsRefused() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        let far = importable("far1", "tune the cache", minutesAgo: 1)
        daemon.addImportable(importable("mac1", "fix the login", minutesAgo: 1))
        daemon.addImportable(far, on: "far")
        let store = TestStore(initialState: ImportSessionFeature.State(projects: [])) {
            ImportSessionFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.sessions?.map(\.id), ["mac1"])
        await store.send(\.binding.host, "far")
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.sessions?.map(\.id), ["far1"])

        _ = try await daemon.client.importSession(far, "far", nil)
        await store.send(\.binding.selection, "far1")
        await store.send(.importTapped)
        await store.receive(\.failed)
        XCTAssertNotNil(store.state.alert)
        XCTAssertFalse(store.state.importing)
        XCTAssertEqual(daemon.sessions.count, 1)
        XCTAssertEqual(daemon.sessions.first?.host, "far")
    }

    private func waiting(_ id: String, requests: [String], hostOffline: Bool = false) -> InboxEntry {
        InboxEntry(
            session: Session(
                id: id, name: id, backend: .scripted, workingDirectory: "/", state: .waitingForApproval,
                pending: requests.map { PendingPermission(requestID: $0, kind: .shell, summary: "make") },
                hostOffline: hostOffline),
            reason: .approval)
    }

    func testAnsweringARequestMovesTheInboxShortcutToTheSessionsNextOne() async throws {
        var state = ProjectsFeature.State()
        state.inbox = [waiting("s1", requests: ["r1", "r2"])]
        let store = TestStore(initialState: state) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = InMemoryDaemon().client
            $0.daemon.approve = { _, _, _ in try await Task.never() }
        }
        store.exhaustivity = .off
        XCTAssertEqual(store.state.inboxShortcut, .permission(entry: "s1", requestID: "r1"))

        let answer = await store.send(.inboxAnswered("s1", request: "r1", approved: true))
        XCTAssertEqual(store.state.inboxShortcut, .permission(entry: "s1", requestID: "r2"))
        await answer.cancel()
    }

    func testTheInboxShortcutAnswersAdminActionsFirstThenTheFirstRowThatCanBeAnswered() {
        let asked = Date(timeIntervalSince1970: 1_794_000_000)
        let session = Session(id: "s0", name: "s0", backend: .scripted, workingDirectory: "/")
        let question = InboxEntry(
            session: session, reason: .question,
            request: AgentRequest(id: "q1", sessionID: "s0", kind: .question, text: "Email or phone?", created: asked))
        let move = InboxEntry(
            session: session, reason: .move,
            request: AgentRequest(
                id: "m1", sessionID: "s0", kind: .move(item: "i1", status: .done), text: "Done.", created: asked))
        let unanswerable = [
            waiting("far", requests: ["r0"], hostOffline: true), InboxEntry(session: session, reason: .failed),
            question,
        ]
        var state = ProjectsFeature.State()

        state.inbox = unanswerable + [InboxEntry(session: session, reason: .reply)]
        XCTAssertNil(state.inboxShortcut)

        state.inbox = unanswerable + [waiting("s1", requests: ["r1"]), move]
        XCTAssertEqual(state.inboxShortcut, .permission(entry: "s1", requestID: "r1"))

        state.inbox = unanswerable + [move, waiting("s1", requests: ["r1"])]
        XCTAssertEqual(state.inboxShortcut, .request(entry: "m1", "m1"))

        state.approvals = [
            AdminApproval(id: "a1", client: "humata mcp", scope: .board, summary: "Add a task", asked: asked),
            AdminApproval(id: "a2", client: "humata mcp", scope: .board, summary: "Add another", asked: asked),
        ]
        XCTAssertEqual(state.inboxShortcut, .approval("a1"))
    }

    func testAFailureThatArrivesWhileASheetIsOpenShowsOnceItCloses() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        var state = ProjectsFeature.State()
        state.sessions = [session]
        let store = TestStore(initialState: state) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.renameTapped(session.id))
        await store.send(.sessionDeleteFailed(session.id, "It is still working."))
        XCTAssertNotNil(store.state.destination?.rename)

        await store.send(.destination(.dismiss))
        XCTAssertNotNil(store.state.destination?.alert)
        await store.send(.destination(.dismiss))
        XCTAssertNil(store.state.destination)
    }

    func testAWorkingSessionsSecondsDontChangeTheSessionsListed() async {
        let project = Project(id: "p1", name: "Humata", repositories: [], created: Self.at(0))
        func working(at minutes: Double) -> Session {
            Session(
                id: "a", name: "a", backend: .scripted, workingDirectory: "/", state: .working, projectID: "p1",
                lastActive: Self.at(minutes))
        }
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Self.at(20))
            $0.notifications = .previewValue
        }
        store.exhaustivity = .off

        await store.send(.loaded(projects: [project], sessions: [working(at: 2.1)], inbox: []))
        let listed = store.state.sessions
        let order = store.state.order

        // Read again seconds later, mid-reply: nothing a row shows changed.
        await store.send(.loaded(projects: [project], sessions: [working(at: 2.6)], inbox: []))
        XCTAssertEqual(store.state.sessions, listed)
        XCTAssertEqual(store.state.order, order)

        // A minute on, its row says so.
        await store.send(.loaded(projects: [project], sessions: [working(at: 3.2)], inbox: []))
        XCTAssertNotEqual(store.state.sessions, listed)
        XCTAssertEqual(store.state.order, order)
    }

    func testALongDiffShowsItsFirstLinesUntilTheRestIsAskedFor() async {
        let long = (1...12_000).map { "+line \($0)\n" }.joined()
        let changed = [RepositoryChanges(name: "app", patch: long), RepositoryChanges(name: "api", patch: "+more\n")]
        let left = Session(id: "a", name: "a", backend: .scripted, workingDirectory: "/")
        let right = Session(id: "b", name: "b", backend: .scripted, workingDirectory: "/")
        let store = TestStore(initialState: CompareFeature.State(left: left, right: right)) {
            CompareFeature()
        } withDependencies: {
            $0.daemon = InMemoryDaemon().client
            $0.daemon.changes = { session, _ in session == "a" ? changed : [RepositoryChanges(name: "app", patch: "")] }
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.leftChanges?.lineCount, 12_001)
        XCTAssertEqual(store.state.leftChanges?.shownCount, 5_000)
        XCTAssertEqual(store.state.leftChanges?.hidden, 7_001)
        XCTAssertEqual(store.state.rightChanges?.isEmpty, true)
        XCTAssertEqual(store.state.rightChanges?.hidden, 0)

        // Asking for one side's rest leaves the other as it was.
        await store.send(.showRestTapped("b"))
        XCTAssertEqual(store.state.leftChanges?.shownCount, 5_000)
        await store.send(.showRestTapped("a"))
        XCTAssertEqual(store.state.leftChanges?.shownCount, 12_001)
        XCTAssertEqual(store.state.leftChanges?.hidden, 0)
        XCTAssertEqual(store.state.leftChanges?.patches.map(\.shown), [12_000, 1])
    }

    func testAKeepThatFailsLeavesBothChangesShownAndCanBePressedAgain() async throws {
        let daemon = InMemoryDaemon()
        let left = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/a"))
        let right = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/b"))
        let refuses = LockIsolated(true)
        var client = daemon.client
        let update = client.update
        client.update = { session, change in
            if refuses.value { throw DaemonDown() }
            return try await update(session, change)
        }
        client.changes = { session, _ in [RepositoryChanges(name: "app", patch: "+from \(session)\n")] }
        let store = TestStore(initialState: CompareFeature.State(left: left, right: right)) {
            CompareFeature()
        } withDependencies: {
            $0.daemon = client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        let (leftChanges, rightChanges) = (store.state.leftChanges, store.state.rightChanges)
        XCTAssertNotNil(leftChanges)
        XCTAssertNotNil(rightChanges)

        await store.send(.keepTapped(left.id))
        await store.receive(\.markFailed)
        XCTAssertNotNil(store.state.keepFailure)
        XCTAssertNil(store.state.failure)
        XCTAssertEqual(store.state.leftChanges, leftChanges)
        XCTAssertEqual(store.state.rightChanges, rightChanges)
        XCTAssertFalse(store.state.marking)

        refuses.setValue(false)
        await store.send(.keepTapped(left.id))
        await store.receive(\.marked)
        XCTAssertNil(store.state.keepFailure)
        XCTAssertTrue(store.state.left.keeper)
        let kept = try await daemon.client.sessions(nil).first { $0.id == left.id }?.keeper
        XCTAssertEqual(kept, true)
    }

    func testSessionsToImportThatCouldntBeReadAreListedOnTryingAgain() async throws {
        let daemon = InMemoryDaemon()
        daemon.addImportable(importable("mac1", "fix the login", minutesAgo: 1))
        let down = LockIsolated(true)
        var client = daemon.client
        let read = client.importable
        client.importable = { host in
            if down.value { throw DaemonDown() }
            return try await read(host)
        }
        let store = TestStore(initialState: ImportSessionFeature.State(projects: [])) {
            ImportSessionFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertNotNil(store.state.loadError)

        down.setValue(false)
        await store.send(.retryTapped)
        await store.receive(\.loaded)
        XCTAssertNil(store.state.loadError)
        XCTAssertEqual(store.state.sessions?.map(\.id), ["mac1"])
    }

    func testTasksThatCantBeReadSaySoInTheSheetAndAreReadAgainOnAsking() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, name: "first", projectID: project.id))
        let answers = LockIsolated(false)
        var client = daemon.client
        let items = client.items
        client.items = { project in
            guard answers.value else { throw DaemonError.unexpected("no answer") }
            return try await items(project)
        }
        let store = TestStore(initialState: MoveSessionFeature.State(session: session)) {
            MoveSessionFeature()
        } withDependencies: {
            $0.daemon = client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.itemsFailed)
        XCTAssertNotNil(store.state.itemsFailure)
        XCTAssertNil(store.state.items)
        // Nothing was moved, so no alert says a move failed.
        XCTAssertNil(store.state.alert)

        answers.setValue(true)
        await store.send(.task)
        XCTAssertNil(store.state.itemsFailure)
        await store.receive(\.itemsLoaded)
        XCTAssertEqual(store.state.items?.map(\.id), [task.id])
    }

    func testListingModelsShowsUntilTheListArrives() async throws {
        let daemon = InMemoryDaemon()
        daemon.setModels([ModelChoice(id: "apple", name: "Apple")], for: .local)
        let (listAsked, asked) = AsyncStream.makeStream(of: Void.self)
        let (listAnswers, answer) = AsyncStream.makeStream(of: Void.self)
        var client = daemon.client
        let listed = client.models
        client.models = { host, backend, config in
            asked.yield()
            for await _ in listAnswers { break }
            return try await listed(host, backend, config)
        }
        var initial = NewSessionFeature.State(project: nil)
        initial.backend = .scripted
        let store = TestStore(initialState: initial) {
            NewSessionFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = client
            $0.continuousClock = ImmediateClock()
        }
        store.exhaustivity = .off
        XCTAssertFalse(store.state.modelsLoading)

        await store.send(\.binding.backend, .local)
        XCTAssertTrue(store.state.modelsLoading)
        for await _ in listAsked { break }
        answer.yield()
        await store.receive(\.modelsLoaded)
        XCTAssertFalse(store.state.modelsLoading)
        XCTAssertEqual(store.state.model, "apple")

        // An agent with nothing to list isn't waited on.
        await store.send(\.binding.backend, .scripted)
        XCTAssertFalse(store.state.modelsLoading)
    }

    /// A daemon with the host "far" whose hosts can't be read while
    /// `down` is true, and a New Session sheet that remembers `host`.
    private func hostStore(
        remembering host: String?, down: LockIsolated<Bool>
    ) async throws -> (InMemoryDaemon, TestStoreOf<NewSessionFeature>) {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        var client = daemon.client
        let read = client.hosts
        client.hosts = {
            if down.value { throw DaemonDown() }
            return try await read()
        }
        var initial = NewSessionFeature.State(project: nil)
        initial.backend = .scripted
        // Picked, so the defaults leave the agent as it is.
        initial.picked = true
        initial.host = host
        let store = TestStore(initialState: initial) {
            NewSessionFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = client
        }
        store.exhaustivity = .off
        return (daemon, store)
    }

    func testARememberedHostIsKeptAndStartWaitsUntilTheHostsAreRead() async throws {
        let down = LockIsolated(true)
        let (daemon, store) = try await hostStore(remembering: "far", down: down)

        await store.send(.task)
        await store.receive(\.hostsFailed)
        XCTAssertEqual(store.state.host, "far")
        XCTAssertNotNil(store.state.hostsFailure)
        XCTAssertFalse(store.state.canStart)
        await store.send(.startTapped)
        XCTAssertTrue(daemon.sessions.isEmpty)

        down.setValue(false)
        await store.send(.hostsRetried)
        await store.receive(\.hostsLoaded)
        XCTAssertEqual(store.state.host, "far")
        XCTAssertNil(store.state.hostsFailure)
        XCTAssertTrue(store.state.canStart)
    }

    func testRunOnThisMacStartsHereWhenTheHostsCantBeRead() async throws {
        let (daemon, store) = try await hostStore(remembering: "far", down: LockIsolated(true))

        await store.send(.task)
        await store.receive(\.hostsFailed)
        await store.send(.thisMacPicked)
        XCTAssertNil(store.state.host)
        XCTAssertNil(store.state.hostsFailure)
        XCTAssertTrue(store.state.canStart)
        await store.send(.startTapped)
        await store.receive(\.delegate.started)
        XCTAssertEqual(daemon.sessions.count, 1)
        XCTAssertNil(daemon.sessions.first?.host)
    }

    func testWithNoHostRememberedAFailedHostsReadLeavesStartAlone() async throws {
        let (_, store) = try await hostStore(remembering: nil, down: LockIsolated(true))

        await store.send(.task)
        await store.receive(\.hostsFailed)
        XCTAssertNotNil(store.state.hostsFailure)
        XCTAssertTrue(store.state.canStart)
    }

    func testTryingAgainFromTheInboxSendsTheMessagesImagesToo() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        let picture = ImageAttachment(mediaType: "image/png", data: Data([1, 2, 3]))
        try await daemon.client.sendImages(session.id, "what is it?", [picture], .enqueue)
        let store = inboxStore(daemon)

        await store.send(.inboxRetryTapped(session.id))
        await store.receive(\.loaded)
        let sent = try await daemon.client.history(session.id).compactMap { event -> [String]? in
            guard case .userMessage("what is it?", let images) = event.kind else { return nil }
            return images
        }
        XCTAssertEqual(sent.count, 2)
        let again = try XCTUnwrap(sent.last)
        XCTAssertEqual(again.count, 1)
        for name in again {
            let read = try await daemon.client.readImage(session.id, name)
            XCTAssertEqual(read, picture)
        }
        XCTAssertEqual(store.state.retrying, [])
    }

    func testTryingAgainFromTheInboxSendsNothingWhenAnImageCantBeRead() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        let picture = ImageAttachment(mediaType: "image/png", data: Data([1, 2, 3]))
        try await daemon.client.sendImages(session.id, "what is it?", [picture], .enqueue)
        let sent = LockIsolated(0)
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.daemon.readImage = { _, _ in throw DaemonDown() }
            $0.daemon.send = { _, _, _ in sent.withValue { $0 += 1 } }
            $0.daemon.sendImages = { _, _, _, _ in sent.withValue { $0 += 1 } }
            $0.notifications.post = { _ in }
        }
        store.exhaustivity = .off

        await store.send(.inboxRetryTapped(session.id))
        await store.receive(\.answerFailed)
        XCTAssertEqual(sent.value, 0)
        XCTAssertNotNil(store.state.destination?.alert)
        // The row is no longer being sent, so it can be tried again.
        XCTAssertEqual(store.state.retrying, [])
        await settle(store)
    }

    func testAnArchiveThatFailsAfterTheAnswerLeavesTheRequestAnswered() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, projectID: project.id, itemID: task.id))
        let move = daemon.addRequest(session.id, .move(item: task.id, status: .done), "Login works.")
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.daemon.update = { _, _ in throw DaemonDown() }
            $0.notifications.post = { _ in }
        }
        store.exhaustivity = .off

        await store.send(.refresh)
        await store.receive(\.loaded)
        await store.send(.requestAnswered(move.id, .approve))
        await store.send(.destination(.presented(.alert(.closeItem(move.id, .approve, archive: true)))))
        await store.receive(\.answerFailed)
        await store.receive(\.loaded)

        let status = try await daemon.client.items(project.id).first?.status
        XCTAssertEqual(status, .done)
        XCTAssertTrue(store.state.answered.contains(move.id))
        XCTAssertFalse(store.state.inbox.contains { $0.request != nil })
        XCTAssertNotNil(store.state.destination?.alert)
        let archived = try await daemon.client.sessions(nil).first { $0.id == session.id }?.archived
        XCTAssertEqual(archived, false)
    }

    func testARepositoryRowSaysWhenItNeedsAFullPath() {
        XCTAssertFalse(RepositoryRow(id: 0).needsFullPath)
        XCTAssertFalse(RepositoryRow(id: 0, path: "  ").needsFullPath)
        XCTAssertFalse(RepositoryRow(id: 0, path: "/code/humata").needsFullPath)
        XCTAssertFalse(RepositoryRow(id: 0, path: "https://github.com/owner/name").needsFullPath)
        XCTAssertTrue(RepositoryRow(id: 0, path: "~/code/humata").needsFullPath)
        XCTAssertTrue(RepositoryRow(id: 0, path: "code/humata").needsFullPath)
    }
}

/// The daemon couldn't be reached.
private struct DaemonDown: Error {}
