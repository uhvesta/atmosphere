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

    func testAnUnreachableDaemonSaysWhatToDoAndRetrying() async {
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
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
        await store.finish()
        XCTAssertEqual(badges.value, [1])
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

    func testTheNextSessionStartsFromWhatTheLastOneDid() async throws {
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
        XCTAssertEqual(next.backend, .claude)
        XCTAssertEqual(next.model, "opus")
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

        // Answered already, from elsewhere: the person is told.
        await store.send(.requestAnswered(move.id, .dismiss))
        await store.receive(\.answerFailed)
        XCTAssertNotNil(store.state.destination)
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
                if case .userMessage(let text) = $0.kind { text } else { nil }
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
}
