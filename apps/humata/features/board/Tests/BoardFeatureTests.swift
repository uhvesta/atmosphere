import ComposableArchitecture
import HumataBoard
import HumataDaemonClient
import HumataProjects
import HumataReview
import HumataSession
import XCTest

@MainActor
final class BoardFeatureTests: XCTestCase {
    private struct Board {
        var daemon: InMemoryDaemon
        var project: Project
        var task: Item
        var subtask: Item
        var other: Item
    }

    /// A project with a task "login" and its subtasks "form" and "copy".
    private func board(
        script: @escaping @Sendable (String) -> [InMemoryDaemon.Step] = { [.say("You said: \($0)")] }
    ) async throws -> Board {
        let daemon = InMemoryDaemon(script: script)
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        let subtask = try await daemon.client.createItem(project.id, ItemFields(parentID: task.id, title: "form"))
        let other = try await daemon.client.createItem(project.id, ItemFields(parentID: task.id, title: "copy"))
        return Board(daemon: daemon, project: project, task: task, subtask: subtask, other: other)
    }

    private func store(_ board: Board) -> TestStoreOf<BoardFeature> {
        let store = TestStore(initialState: BoardFeature.State(project: board.project)) {
            BoardFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
        }
        store.exhaustivity = .off
        return store
    }

    private func titles(_ items: [Item]) -> [String] { items.map(\.title) }

    /// The board, with `backend` and `model` as every project's New Session
    /// defaults.
    private func store(_ board: Board, defaultBackend backend: Backend, model: String = "") async throws
        -> TestStoreOf<BoardFeature>
    {
        _ = try await board.daemon.client.setSessionDefaults(
            nil, SessionDefaults(backend: backend, model: model.isEmpty ? nil : model))
        let store = TestStore(initialState: BoardFeature.State(project: board.project)) {
            BoardFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
        }
        store.exhaustivity = .off
        return store
    }

    func testAReadOfTheBoardWithNothingNewIsNotTakenIn() async throws {
        let board = try await board()
        let clock = TestClock()
        let made = LockIsolated(0)
        let taken = LockIsolated(0)
        var client = board.daemon.client
        let items = client.items
        client.items = { project in
            made.withValue { $0 += 1 }
            return try await items(project)
        }
        let counted = CombineReducers {
            Reduce<BoardFeature.State, BoardFeature.Action> { _, action in
                if case .loaded = action { taken.withValue { $0 += 1 } }
                return .none
            }
            BoardFeature()
        }
        let store = TestStore(initialState: BoardFeature.State(project: board.project)) {
            counted
        } withDependencies: { [client] in
            $0.daemon = client
            $0.continuousClock = clock
        }
        store.exhaustivity = .off

        let task = await store.send(.task)
        await store.receive(\.loaded)
        // The reads are made in turn: by the third, the second, of the
        // same board as the first, is over.
        while made.value < 3 { await clock.advance(by: .seconds(2)) }
        let added = try await board.daemon.client.createItem(board.project.id, ItemFields(title: "signup"))
        await clock.advance(by: .seconds(2))
        await store.receive(\.loaded)
        XCTAssertNotNil(store.state.items[id: added.id])
        XCTAssertEqual(taken.value, 2)
        await task.cancel()
    }

    func testStartWorkStartsASessionOnTheCardWithTheDefaultsAndStaysOnTheBoard() async throws {
        let board = try await board()
        let store = try await store(board, defaultBackend: .scripted)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.startWorkTapped(board.task.id))
        await store.receive(\.loaded)

        let session = try XCTUnwrap(board.daemon.sessions.first)
        XCTAssertEqual(board.daemon.sessions.count, 1)
        XCTAssertEqual(session.itemID, board.task.id)
        XCTAssertEqual(session.backend, .scripted)
        XCTAssertEqual(session.name, "login")
        let history = try await board.daemon.client.history(session.id)
        let sent = history.filter { if case .userMessage = $0.kind { true } else { false } }
        XCTAssertEqual(sent.count, 1)
        XCTAssertNil(store.state.destination)
    }

    func testTappingStartWorkTwiceStartsOneSession() async throws {
        let board = try await board()
        let store = try await store(board, defaultBackend: .scripted)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.startWorkTapped(board.task.id))
        XCTAssertTrue(store.state.starting.contains(board.task.id))
        await store.receive(\.startFinished)
        await store.receive(\.loaded)
        XCTAssertTrue(store.state.starting.isEmpty)
        XCTAssertEqual(board.daemon.sessions.count, 1)

        // A tap while it's still starting does nothing.
        var starting = store.state
        starting.starting = [board.task.id]
        let again = TestStore(initialState: starting) {
            BoardFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
        }
        again.exhaustivity = .off
        await again.send(.startWorkTapped(board.task.id))
        await again.finish()
        XCTAssertEqual(board.daemon.sessions.count, 1)
    }

    func testAFailedChangeStaysShownThroughAReadUntilClosed() async throws {
        let board = try await board()
        let store = store(board)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.deleteTapped(board.task.id))
        await store.send(.destination(.presented(.alert(.delete(board.task.id)))))
        await store.receive(\.actionFailed)
        XCTAssertNotNil(store.state.actionFailure)

        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertNotNil(store.state.actionFailure)

        await store.send(.actionFailureDismissed)
        XCTAssertNil(store.state.actionFailure)
    }

    func testCancellingAChangedSpecAsksAndAnUnchangedOneClosesAtOnce() async throws {
        let board = try await board()
        let store = TestStore(
            initialState: ItemFeature.State(
                project: board.project, item: board.task, parent: nil, subtasks: [], sessions: [])
        ) {
            ItemFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
        }
        store.exhaustivity = .off

        await store.send(.editTapped)
        await store.send(.cancelEditTapped)
        XCTAssertNil(store.state.draft)
        XCTAssertNil(store.state.destination)

        await store.send(.editTapped)
        await store.send(\.binding.draft, ItemFeature.Draft(title: "login", spec: "# Steps"))
        await store.send(.cancelEditTapped)
        XCTAssertNotNil(store.state.draft, "kept until the discard is confirmed")
        XCTAssertNotNil(store.state.destination?.alert)

        await store.send(.destination(.presented(.alert(.discard))))
        XCTAssertNil(store.state.draft)
    }

    func testAnItemsSessionsListThoseWaitingFirstAndArchivedLast() async throws {
        let board = try await board()
        func session(_ id: String, _ state: SessionState = .idle, archived: Bool = false) -> Session {
            Session(id: id, name: id, backend: .scripted, workingDirectory: "/work", state: state, archived: archived)
        }
        let page = ItemFeature.State(
            project: board.project, item: board.task, parent: nil, subtasks: [],
            sessions: [
                session("old", archived: true), session("idle"), session("busy", .working),
                session("blocked", .waitingForApproval), session("failed", .failed),
            ])
        XCTAssertEqual(page.sortedSessions.map(\.id), ["blocked", "failed", "busy", "idle", "old"])
    }

    func testStartWorkAsksForAModelWhenTheDefaultAgentIsALocalOneWithNone() async throws {
        let board = try await board()
        let store = try await store(board, defaultBackend: .local)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.startWorkTapped(board.subtask.id))
        await store.receive(\.startNeedsModel)

        XCTAssertTrue(board.daemon.sessions.isEmpty)
        guard case .item(let page) = store.state.destination, case .newSession = page.destination else {
            return XCTFail("Expected the item's page asking for a model")
        }
        XCTAssertEqual(page.item.id, board.subtask.id)
    }

    func testASessionWaitingOnASubtaskMovesItAndItsTask() async throws {
        let board = try await board { _ in [.run(command: "make test", output: "ok")] }
        let session = try await board.daemon.client.start(StartRequest(backend: .scripted, itemID: board.subtask.id))
        try await board.daemon.client.send(session.id, "test it", .enqueue)
        let store = store(board)

        await store.send(.refresh)
        await store.receive(\.loaded)

        XCTAssertEqual(titles(store.state.cards(in: .waitingOnYou)), ["login", "form"])
        XCTAssertEqual(titles(store.state.cards(in: .todo)), ["copy"])
        let subtask = try XCTUnwrap(store.state.items[id: board.subtask.id])
        XCTAssertEqual(store.state.sessions(on: subtask).map(\.id), [session.id])
    }

    func testDroppingACardOnDoneKeepsItThereThroughItsSessions() async throws {
        let board = try await board()
        let session = try await board.daemon.client.start(StartRequest(backend: .scripted, itemID: board.task.id))
        try await board.daemon.client.send(session.id, "hello", .enqueue)
        let store = store(board)
        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(titles(store.state.cards(in: .inReview)), ["login"])

        await store.send(.dropped(board.task.id, on: .done))
        await store.send(.destination(.presented(.alert(.keep(board.task.id, .done)))))
        await store.receive(\.moved)
        await store.receive(\.loaded)

        XCTAssertEqual(titles(store.state.cards(in: .done)), ["login"])
        XCTAssertEqual(titles(store.state.cards(in: .inReview)), [])
    }

    func testADroppedCardShowsInItsColumnThroughAnOlderRead() async throws {
        let board = try await board()
        // The daemon answers the drop only once the older read is in.
        let (answer, release) = AsyncStream<Void>.makeStream()
        let store = TestStore(initialState: BoardFeature.State(project: board.project)) {
            BoardFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
            $0.daemon.updateItem = { id, update in
                for await _ in answer { break }
                return try await board.daemon.client.updateItem(id, update)
            }
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.loaded)
        let before = try await board.daemon.client.items(board.project.id)

        await store.send(.dropped(board.other.id, on: .inProgress))
        XCTAssertEqual(store.state.items[id: board.other.id]?.fields.status, .inProgress)
        let sessions = Array(store.state.sessions)
        await store.send(.loaded(items: before, sessions: sessions))
        XCTAssertEqual(store.state.items[id: board.other.id]?.fields.status, .inProgress)
        release.yield()
        await store.receive(\.moved)
        XCTAssertEqual(store.state.moving, [:])
    }

    func testARefusedDropPutsTheCardBack() async throws {
        let board = try await board()
        let store = TestStore(initialState: BoardFeature.State(project: board.project)) {
            BoardFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
            $0.daemon.updateItem = { _, _ in throw DaemonError.disconnected }
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.loaded)
        let was = try XCTUnwrap(store.state.items[id: board.other.id]?.fields.status)

        await store.send(.dropped(board.other.id, on: .done))
        await store.receive(\.moveFailed)
        XCTAssertEqual(store.state.items[id: board.other.id]?.fields.status, was)
        XCTAssertNotNil(store.state.actionFailure)
        XCTAssertEqual(store.state.moving, [:])
    }

    func testASecondUndoWhileTheFirstRunsUndoesNothingMore() async throws {
        let board = try await board()
        let reverts = LockIsolated(0)
        let store = TestStore(initialState: BoardFeature.State(project: board.project)) {
            BoardFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
            // The first undo is still going when the second tap comes.
            $0.daemon.revert = { _ in
                reverts.withValue { $0 += 1 }
                return try await Task.never()
            }
            $0.continuousClock = ImmediateClock()
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.loaded)
        let newest = try await board.daemon.client.boardLog(board.project.id, nil, 1).first?.id
        await store.send(.logStarted(newest: newest))
        await store.send(.dropped(board.task.id, on: .inProgress))
        await store.receive(\.moved)
        await store.send(.dropped(board.other.id, on: .done))
        await store.receive(\.moved)
        await store.skipReceivedActions(strict: false)

        let first = await store.send(.undoTapped)
        XCTAssertTrue(store.state.undoing)
        await store.send(.undoTapped)
        await Task.megaYield()
        XCTAssertEqual(reverts.value, 1)
        await first.cancel()
    }

    func testClosingACardOffersToStopAndArchiveItsSessions() async throws {
        let board = try await board { _ in [.run(command: "make", output: "")] }
        let working = try await board.daemon.client.start(StartRequest(backend: .scripted, itemID: board.task.id))
        let idle = try await board.daemon.client.start(StartRequest(backend: .scripted, itemID: board.task.id))
        try await board.daemon.client.send(working.id, "build it", .enqueue)
        let store = store(board)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.dropped(board.task.id, on: .done))
        XCTAssertNotNil(store.state.destination?.alert)
        await store.send(.destination(.presented(.alert(.stopAndArchive(board.task.id, .done)))))
        await store.receive(\.moved)

        let sessions = try await board.daemon.client.sessions(nil)
        XCTAssertEqual(Set(sessions.filter(\.archived).map(\.id)), [working.id, idle.id])
        XCTAssertFalse(sessions.contains { $0.state == .working || $0.state == .waitingForApproval })
        XCTAssertEqual(store.state.items[id: board.task.id]?.fields.status, .done)
    }

    func testKeepingTheSessionsClosesTheCardAndLeavesThemBe() async throws {
        let board = try await board()
        let session = try await board.daemon.client.start(StartRequest(backend: .scripted, itemID: board.task.id))
        let store = store(board)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.dropped(board.task.id, on: .wontDo))
        await store.send(.destination(.presented(.alert(.keep(board.task.id, .wontDo)))))
        await store.receive(\.moved)

        let sessions = try await board.daemon.client.sessions(nil)
        XCTAssertEqual(sessions.map(\.id), [session.id])
        XCTAssertFalse(sessions.contains(where: \.archived))
        XCTAssertEqual(store.state.items[id: board.task.id]?.fields.status, .wontDo)
    }

    func testDeletingACardRemovesItAndATaskWithSubtasksStays() async throws {
        let board = try await board()
        let store = store(board)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.deleteTapped(board.task.id))
        await store.send(.destination(.presented(.alert(.delete(board.task.id)))))
        await store.receive(\.actionFailed)
        XCTAssertNotNil(store.state.items[id: board.task.id])

        await store.send(.deleteTapped(board.other.id))
        await store.send(.destination(.presented(.alert(.delete(board.other.id)))))
        await store.receive(\.loaded)
        XCTAssertEqual(titles(store.state.cards(in: .todo)), ["login", "form"])
    }

    func testATasksPageOpensItsSubtasksAndBack() async throws {
        let board = try await board()
        let store = store(board)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.cardTapped(board.task.id))
        await store.send(\.destination.item.itemTapped, board.subtask.id)
        await store.receive(\.destination.item.delegate.openItem)
        await store.receive(\.cardTapped)
        XCTAssertEqual(store.state.destination?.item?.item.id, board.subtask.id)
        XCTAssertEqual(store.state.destination?.item?.parent?.id, board.task.id)

        await store.send(\.destination.item.itemTapped, board.task.id)
        await store.receive(\.cardTapped)
        XCTAssertEqual(store.state.destination?.item?.item.id, board.task.id)
    }

    func testUnsentSpecCommentsAreThereWhenTheTasksPageOpensAgain() async throws {
        let board = try await board()
        let store = TestStore(initialState: BoardFeature.State(project: board.project)) {
            BoardFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.loaded)
        let block = MarkdownBlock(kind: .paragraph, startLine: 1, endLine: 1, text: "Email and password.")

        await store.send(.cardTapped(board.task.id))
        await store.send(\.destination.item.comments.blockTapped, block)
        await store.send(\.destination.item.comments.binding.comment, "And a magic link.")
        await store.send(\.destination.item.comments.commentSaved)
        await store.send(.destination(.dismiss))
        XCTAssertNil(store.state.destination)

        // Another page opened in between has none of them.
        await store.send(.cardTapped(board.subtask.id))
        XCTAssertEqual(store.state.destination?.item?.comments.drafts.isEmpty, true)
        await store.send(.destination(.dismiss))

        await store.send(.cardTapped(board.task.id))
        XCTAssertEqual(store.state.destination?.item?.comments.drafts.map(\.text), ["And a magic link."])
    }

    func testSpecCommentsBeingSentWhenTheTasksPageClosesAreThereWhenItOpensAgain() async throws {
        let board = try await board()
        let store = TestStore(initialState: BoardFeature.State(project: board.project)) {
            BoardFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
            $0.uuid = .incrementing
            // The send is still going when the page closes.
            $0.daemon.send = { _, _, _ in try await Task.never() }
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.loaded)
        let block = MarkdownBlock(kind: .paragraph, startLine: 1, endLine: 1, text: "Email and password.")

        await store.send(.cardTapped(board.task.id))
        await store.send(\.destination.item.comments.blockTapped, block)
        await store.send(\.destination.item.comments.binding.comment, "And a magic link.")
        await store.send(\.destination.item.comments.commentSaved)
        await store.send(\.destination.item.comments.sendTapped, "s1")
        XCTAssertEqual(store.state.destination?.item?.comments.sending, true)
        await store.send(.destination(.dismiss))
        XCTAssertNil(store.state.destination)

        await store.send(.cardTapped(board.task.id))
        XCTAssertEqual(store.state.destination?.item?.comments.drafts.map(\.text), ["And a magic link."])
        XCTAssertEqual(store.state.destination?.item?.comments.sending, false)
    }

    func testSpecCommentsGoOnlyToSessionsStillWorking() async throws {
        let board = try await board()
        let old = try await board.daemon.client.start(StartRequest(backend: .scripted, itemID: board.task.id))
        let live = try await board.daemon.client.start(StartRequest(backend: .scripted, itemID: board.task.id))
        _ = try await board.daemon.client.update(old.id, SessionUpdate(archived: true))
        let store = store(board)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.cardTapped(board.task.id))
        let page = try XCTUnwrap(store.state.destination?.item)
        XCTAssertEqual(Set(page.sessions.map(\.id)), [old.id, live.id])
        XCTAssertEqual(page.commentTargets.map(\.id), [live.id])
    }

    func testANewSubtaskShowsOnTheBoard() async throws {
        let board = try await board()
        let store = store(board)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.addTapped(parent: board.task.id))
        await store.send(\.destination.newItem.binding.title, "tests")
        await store.send(\.destination.newItem.createTapped)
        await store.receive(\.destination.newItem.delegate.created)
        await store.receive(\.loaded)

        XCTAssertNil(store.state.destination)
        let added = try XCTUnwrap(store.state.items.first { $0.title == "tests" })
        XCTAssertEqual(added.parentID, board.task.id)
    }

    func testEditingTheSpecOnTheItemPageSavesIt() async throws {
        let board = try await board()
        let store = store(board)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.cardTapped(board.subtask.id))
        XCTAssertEqual(store.state.destination?.item?.parent?.id, board.task.id)
        await store.send(\.destination.item.editTapped)
        await store.send(\.destination.item.binding.draft, ItemFeature.Draft(title: "form", spec: "# Fields\n- email"))
        // Saved from the preview, what was written is kept.
        await store.send(
            \.destination.item.binding.draft,
            ItemFeature.Draft(title: "form", spec: "# Fields\n- email", previewing: true))
        await store.send(\.destination.item.saveTapped)
        await store.receive(\.destination.item.saved)

        XCTAssertNil(store.state.destination?.item?.draft)
        let items = try await board.daemon.client.items(board.project.id)
        XCTAssertEqual(items.first { $0.id == board.subtask.id }?.spec, "# Fields\n- email")
    }

    func testPickingAPriorityWhileEditingKeepsTheDraft() async throws {
        let board = try await board()
        var page = ItemFeature.State(
            project: board.project, item: board.task, parent: nil, subtasks: [], sessions: [])
        page.draft = ItemFeature.Draft(title: "login page", spec: "half written")
        let store = TestStore(initialState: page) {
            ItemFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
        }
        store.exhaustivity = .off

        await store.send(.priorityPicked(.high))
        await store.receive(\.updated)

        XCTAssertEqual(store.state.item.priority, .high)
        XCTAssertEqual(store.state.draft?.spec, "half written")
    }

    func testAnEmptyTitleIsNotSaved() async throws {
        let board = try await board()
        var page = ItemFeature.State(
            project: board.project, item: board.task, parent: nil, subtasks: [], sessions: [])
        page.draft = ItemFeature.Draft(title: "  ", spec: "")
        let store = TestStore(initialState: page) {
            ItemFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
        }
        store.exhaustivity = .off

        await store.send(.saveTapped)
        await store.receive(\.failed)

        XCTAssertNotNil(store.state.destination?.alert)
        let items = try await board.daemon.client.items(board.project.id)
        XCTAssertEqual(items.first { $0.id == board.task.id }?.title, "login")
    }

    func testStartingASessionOnASubtaskOpensIt() async throws {
        let board = try await board()
        let store = store(board)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.cardTapped(board.subtask.id))
        await store.send(\.destination.item.startSessionTapped)
        await store.send(\.destination.item.destination.newSession.startTapped)
        await store.receive(\.destination.item.destination.newSession.delegate.started)
        await store.receive(\.destination.item.delegate.openSession)
        await store.receive(\.delegate.openSession)

        XCTAssertNil(store.state.destination)
        let session = try XCTUnwrap(board.daemon.sessions.last)
        XCTAssertEqual(session.itemID, board.subtask.id)
        XCTAssertEqual(session.projectID, board.project.id)
    }

    func testRevertingAnEditPutsItBack() async throws {
        let board = try await board()
        _ = try await board.daemon.client.updateItem(board.task.id, ItemUpdate(title: "sign in"))
        let store = store(board)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.logTapped)
        await store.send(\.destination.log.task)
        await store.receive(\.destination.log.loaded)
        let edit = try XCTUnwrap(store.state.destination?.log?.changes.first)
        await store.send(\.destination.log.revertTapped, edit.id)
        await store.receive(\.destination.log.reverted)
        // The log re-reads itself and the board refreshes, in either order.
        await store.skipReceivedActions(strict: false)
        await store.send(.refresh)
        await store.receive(\.loaded)

        XCTAssertEqual(store.state.items[id: board.task.id]?.title, "login")
        let log = try await board.daemon.client.boardLog(board.project.id, nil, nil)
        XCTAssertEqual(log.first?.reverts, edit.id)
    }

    func testRevertingAnEditKeepsAFieldChangedSinceAndSaysSo() async throws {
        let board = try await board()
        _ = try await board.daemon.client.updateItem(board.task.id, ItemUpdate(title: "sign in", priority: .high))
        _ = try await board.daemon.client.updateItem(board.task.id, ItemUpdate(title: "log in"))
        let store = store(board)

        await store.send(.logTapped)
        await store.send(\.destination.log.task)
        await store.receive(\.destination.log.loaded)
        let edit = try XCTUnwrap(store.state.destination?.log?.changes.first { $0.after?.title == "sign in" })
        await store.send(\.destination.log.revertTapped, edit.id)
        await store.receive(\.destination.log.reverted)

        XCTAssertNotNil(store.state.destination?.log?.alert)
        let item = try await board.daemon.client.items(board.project.id).first { $0.id == board.task.id }
        XCTAssertEqual(item?.title, "log in")
        XCTAssertEqual(item?.priority, Priority.none)
    }

    func testTheJournalSaysWhoMadeEachChangeAndFiltersByThem() async throws {
        let board = try await board()
        let time = Date(timeIntervalSince1970: 0)
        let fields = ItemFields(title: "login")
        let changes = [
            BoardChange(id: 3, itemID: board.task.id, time: time, before: nil, after: fields, author: .mcp),
            BoardChange(
                id: 2, itemID: board.task.id, time: time, before: fields, after: fields, author: .session("s1")),
            BoardChange(id: 1, itemID: board.task.id, time: time, before: nil, after: fields, author: .app),
            BoardChange(id: 0, itemID: board.task.id, time: time, before: fields, after: nil, author: .app),
        ]
        let store = TestStore(
            initialState: BoardLogFeature.State(project: board.project, items: [])
        ) {
            BoardLogFeature()
        }
        store.exhaustivity = .off

        await store.send(.loaded(changes))
        XCTAssertEqual(store.state.authors, [.mcp, .session("s1"), .app])
        await store.send(.binding(.set(\.author, .app)))
        XCTAssertEqual(store.state.shown.map(\.id), [1, 0])

        // An author whose changes are gone stops filtering.
        await store.send(.loaded(Array(changes.prefix(2))))
        XCTAssertNil(store.state.author)
        XCTAssertEqual(store.state.shown.count, 2)
    }

    func testUndoTakesBackTheLastChangesMadeHereOneByOne() async throws {
        let board = try await board()
        let store = TestStore(initialState: BoardFeature.State(project: board.project)) {
            BoardFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
            $0.continuousClock = ImmediateClock()
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.loaded)
        // Changes from before the board opened aren't offered.
        let newest = try await board.daemon.client.boardLog(board.project.id, nil, 1).first?.id
        await store.send(.logStarted(newest: newest))
        await store.send(.undoTapped)
        let before = try await board.daemon.client.items(board.project.id)
        XCTAssertEqual(before.count, 3)

        await store.send(.dropped(board.task.id, on: .inProgress))
        await store.receive(\.moved)
        await store.receive(\.offerUndo)
        await store.skipReceivedActions(strict: false)
        await store.send(.dropped(board.other.id, on: .done))
        await store.receive(\.moved)
        await store.receive(\.offerUndo)
        await store.skipReceivedActions(strict: false)

        await store.send(.undoTapped)
        await store.receive(\.undone)
        await store.send(.undoTapped)
        await store.receive(\.undone)
        await store.skipReceivedActions(strict: false)

        let items = try await board.daemon.client.items(board.project.id)
        XCTAssertEqual(items.first { $0.id == board.task.id }?.fields.status, .todo)
        XCTAssertEqual(items.first { $0.id == board.other.id }?.fields.status, .todo)
        XCTAssertNil(store.state.undo)
    }

    func testRemovingAnItemWithSubtasksSaysWhy() async throws {
        let board = try await board()
        let store = store(board)
        await store.send(.logTapped)
        await store.send(\.destination.log.task)
        await store.receive(\.destination.log.loaded)
        let added = try XCTUnwrap(store.state.destination?.log?.changes.first { $0.itemID == board.task.id })

        await store.send(\.destination.log.revertTapped, added.id)
        await store.receive(\.destination.log.failed)

        XCTAssertNotNil(store.state.destination?.log?.alert)
        let items = try await board.daemon.client.items(board.project.id)
        XCTAssertEqual(items.count, 3)
    }

    func testABoardOpenedAtAnItemShowsItsPageOnceRead() async throws {
        let board = try await board()
        let store = TestStore(initialState: BoardFeature.State(project: board.project, opening: board.subtask.id)) {
            BoardFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
        }
        store.exhaustivity = .off

        await store.send(.refresh)
        await store.receive(\.loaded)
        await store.receive(\.cardTapped)
        guard case .item(let page)? = store.state.destination else {
            return XCTFail("no item page: \(String(describing: store.state.destination))")
        }
        XCTAssertEqual(page.item.id, board.subtask.id)
        XCTAssertNil(store.state.opening)
    }

    func testIssuesAreImportedAsTasksOnce() async throws {
        let board = try await board()
        board.daemon.openIssues(
            [.init(number: 3, title: "Crash on launch", body: "It crashes.")], in: board.project.id)
        let store = store(board)

        await store.send(.importIssuesTapped)
        await store.receive(\.imported)
        await store.receive(\.loaded)
        XCTAssertEqual(titles(store.state.cards(in: .todo)).filter { $0 == "Crash on launch" }.count, 1)

        await store.send(.importIssuesTapped)
        await store.receive(\.imported)
        XCTAssertEqual(store.state.importing, false)
        let items = try await board.daemon.client.items(board.project.id)
        XCTAssertEqual(items.filter { $0.title == "Crash on launch" }.count, 1)
    }

    func testATaskPageShowsItsPullRequestsAndTheirMerge() async throws {
        let board = try await board()
        let session = try await board.daemon.client.start(StartRequest(backend: .scripted, itemID: board.task.id))
        _ = try await board.daemon.client.land(
            session.id, LandRequest(branch: "login", message: "Log in", openPullRequests: true))
        let items = try await board.daemon.client.items(board.project.id)
        let item = try XCTUnwrap(items.first { $0.id == board.task.id })
        let store = TestStore(
            initialState: ItemFeature.State(
                project: board.project, item: item, parent: nil, subtasks: [], sessions: [session])
        ) {
            ItemFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.pullRequestsRead)
        XCTAssertEqual(store.state.pullRequests.map(\.state), [.open])

        try board.daemon.mergePullRequests(of: session.id)
        await store.send(.refreshPullRequestsTapped)
        await store.receive(\.pullRequestsRead)
        XCTAssertEqual(store.state.pullRequests.map(\.state), [.merged])
        let merged = try await board.daemon.client.items(board.project.id).first { $0.id == board.task.id }
        XCTAssertEqual(merged?.status, .done)
        await store.finish()
    }

    func testATaskPageSaysWhenPullRequestsAreReadOnlyOnRefresh() async throws {
        let board = try await board()
        let session = try await board.daemon.client.start(StartRequest(backend: .scripted, itemID: board.task.id))
        _ = try await board.daemon.client.land(
            session.id, LandRequest(branch: "login", message: "Log in", openPullRequests: true))
        _ = try await board.daemon.client.setPullRefresh(PullRefresh(automatic: false, everyMinutes: 2))
        let items = try await board.daemon.client.items(board.project.id)
        let item = try XCTUnwrap(items.first { $0.id == board.task.id })
        let store = TestStore(
            initialState: ItemFeature.State(
                project: board.project, item: item, parent: nil, subtasks: [], sessions: [session])
        ) {
            ItemFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.pullRefreshRead)
        XCTAssertTrue(store.state.pullsReadOnRefreshOnly)

        // Refresh still reads them.
        try board.daemon.mergePullRequests(of: session.id)
        await store.send(.refreshPullRequestsTapped)
        await store.receive(\.pullRequestsRead)
        XCTAssertEqual(store.state.pullRequests.map(\.state), [.merged])
        await store.finish()
    }

    func testARefreshThatFailsStillShowsThePullRequestsThatDidRefresh() async throws {
        let board = try await board()
        let session = try await board.daemon.client.start(StartRequest(backend: .scripted, itemID: board.task.id))
        _ = try await board.daemon.client.land(
            session.id, LandRequest(branch: "login", message: "Log in", openPullRequests: true))
        let items = try await board.daemon.client.items(board.project.id)
        let item = try XCTUnwrap(items.first { $0.id == board.task.id })
        var client = board.daemon.client
        let kept = client.pullRequests
        client.pullRequests = { of, refresh in
            if refresh { throw URLError(.notConnectedToInternet) }
            return try await kept(of, false)
        }
        let store = TestStore(
            initialState: ItemFeature.State(
                project: board.project, item: item, parent: nil, subtasks: [], sessions: [session])
        ) {
            ItemFeature()
        } withDependencies: {
            $0.daemon = client
        }
        store.exhaustivity = .off

        try board.daemon.mergePullRequests(of: session.id)
        await store.send(.refreshPullRequestsTapped)
        await store.receive(\.pullRequestsRead)
        await store.receive(\.failed)
        XCTAssertEqual(store.state.pullRequests.map(\.state), [.merged])
        XCTAssertFalse(store.state.readingPullRequests)
        await store.finish()
    }

    func testAnOutsideAgentsProposalReachesTheBoardOnlyOnceApplied() async throws {
        let board = try await board()
        board.daemon.proposeFromOutside(
            Proposal(id: "p1", title: "signup", spec: "A signup form"), for: board.project.id)
        board.daemon.proposeFromOutside(Proposal(id: "p2", title: "logout"), for: board.project.id)
        let store = store(board)
        await store.send(.proposals(.refresh))
        await store.receive(\.proposals.listed)
        XCTAssertEqual(store.state.proposals.pending.map(\.title), ["signup", "logout"])
        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertFalse(titles(Array(store.state.items)).contains("signup"))

        await store.send(.proposals(.applyTapped("p1")))
        await store.receive(\.proposals.decided)
        await store.receive(\.loaded)
        XCTAssertTrue(titles(Array(store.state.items)).contains("signup"))

        await store.send(.proposals(.declineTapped("p2")))
        await store.receive(\.proposals.decided)
        XCTAssertTrue(store.state.proposals.pending.isEmpty)
        XCTAssertFalse(titles(Array(store.state.items)).contains("logout"))
        await store.finish()
    }

    private func page(_ board: Board, sessions: [Session] = []) -> TestStoreOf<ItemFeature> {
        let store = TestStore(
            initialState: ItemFeature.State(
                project: board.project, item: board.task, parent: nil, subtasks: [], sessions: sessions)
        ) {
            ItemFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
        }
        store.exhaustivity = .off
        return store
    }

    func testStartWorkOnATasksPageStartsFromEveryProjectsDefaults() async throws {
        let board = try await board()
        _ = try await board.daemon.client.setSessionDefaults(nil, SessionDefaults(backend: .scripted, model: "tiny"))
        let store = page(board)

        await store.send(.startWorkTapped)
        XCTAssertTrue(store.state.startingWork)
        await store.receive(\.workStarted)
        await store.receive(\.delegate.changed)

        XCTAssertFalse(store.state.startingWork)
        XCTAssertEqual(board.daemon.sessions.count, 1)
        let session = try XCTUnwrap(board.daemon.sessions.first)
        XCTAssertEqual(session.itemID, board.task.id)
        XCTAssertEqual(session.backend, .scripted)
        XCTAssertEqual(session.model, "tiny")
    }

    func testStartWorkOnATasksPageAsksForAModelWhenTheDefaultAgentIsALocalOneWithNone() async throws {
        let board = try await board()
        _ = try await board.daemon.client.setSessionDefaults(nil, SessionDefaults(backend: .local))
        let store = page(board)

        await store.send(.startWorkTapped)
        await store.receive(\.workStarted)
        await store.receive(\.startNeedsModel)

        XCTAssertTrue(board.daemon.sessions.isEmpty)
        XCTAssertFalse(store.state.startingWork)
        XCTAssertEqual(store.state.destination?.newSession?.backend, .local)
    }

    func testTappingTheCardOfTheOpenPageKeepsWhatIsBeingWrittenThere() async throws {
        let board = try await board()
        let store = store(board)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.cardTapped(board.task.id))
        await store.send(\.destination.item.editTapped)
        await store.send(.cardTapped(board.task.id))
        XCTAssertNotNil(store.state.destination?.item?.draft)

        await store.send(.cardTapped(board.subtask.id))
        XCTAssertEqual(store.state.destination?.item?.item.id, board.subtask.id)
    }

    func testAnImportThatEndsWhileATasksPageIsOpenSaysSoOnceItCloses() async throws {
        let board = try await board()
        board.daemon.openIssues(
            [.init(number: 3, title: "Crash on launch", body: "It crashes.")], in: board.project.id)
        let store = store(board)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.cardTapped(board.task.id))
        await store.send(.importIssuesTapped)
        await store.receive(\.imported)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.destination?.item?.item.id, board.task.id)

        await store.send(.destination(.dismiss))
        XCTAssertNotNil(store.state.destination?.alert)
        await store.send(.destination(.dismiss))
        XCTAssertNil(store.state.destination)
    }

    func testAFailureOnATasksPageWaitsForTheSheetOpenOverIt() async throws {
        let board = try await board()
        let store = page(board)

        await store.send(.addSubtaskTapped)
        await store.send(.startWorkFailed("The agent isn't installed."))
        XCTAssertNotNil(store.state.destination?.newSubtask)

        await store.send(.destination(.dismiss))
        XCTAssertNotNil(store.state.destination?.alert)
        await store.send(.destination(.dismiss))
        XCTAssertNil(store.state.destination)

        // With nothing open over the page, it shows at once.
        await store.send(.failed("The daemon isn't running."))
        XCTAssertNotNil(store.state.destination?.alert)
    }

    func testATaskPageCountsWhatEachOfItsSessionsChanged() async throws {
        let board = try await board()
        let first = try await board.daemon.client.start(StartRequest(backend: .scripted, itemID: board.task.id))
        let second = try await board.daemon.client.start(StartRequest(backend: .scripted, itemID: board.task.id))
        let store = page(board, sessions: [first, second])

        await store.send(.task)
        await store.receive(\.changesRead)
        await store.receive(\.changesRead)
        XCTAssertEqual(Set(store.state.changedFiles.keys), [first.id, second.id])
        await store.finish()
    }

    func testAPageOpenedFromAnotherTakesNoneOfItsAnswers() async throws {
        let board = try await board()
        let session = try await board.daemon.client.start(StartRequest(backend: .scripted, itemID: board.task.id))
        _ = try await board.daemon.client.land(
            session.id, LandRequest(branch: "login", message: "Log in", openPullRequests: true))
        // The task's pull requests are answered only once its subtask's page
        // is open in its place.
        let (answer, release) = AsyncStream<Void>.makeStream()
        var client = board.daemon.client
        let kept = client.pullRequests
        client.pullRequests = { of, refresh in
            for await _ in answer { break }
            return try await kept(of, refresh)
        }
        let store = TestStore(initialState: BoardFeature.State(project: board.project)) {
            BoardFeature()
        } withDependencies: {
            $0.daemon = client
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.cardTapped(board.task.id))
        await store.send(\.destination.item.task)
        await store.send(\.destination.item.itemTapped, board.subtask.id)
        await store.receive(\.cardTapped)
        XCTAssertEqual(store.state.destination?.item?.item.id, board.subtask.id)

        release.yield()
        await Task.megaYield()
        XCTAssertEqual(store.state.destination?.item?.pullRequests.isEmpty, true)
    }

    func testAReadThatDiffersOnlyInWhenASessionLastDidSomethingChangesNothing() async throws {
        let board = try await board()
        _ = try await board.daemon.client.start(StartRequest(backend: .scripted, itemID: board.task.id))
        let store = store(board)
        await store.send(.refresh)
        await store.receive(\.loaded)
        let shown = store.state
        let items = Array(shown.items)
        var session = try XCTUnwrap(shown.sessions.first)

        session.lastActive = Date(timeIntervalSince1970: 99)
        session.lastMessage = "Still going."
        await store.send(.loaded(items: items, sessions: [session]))
        XCTAssertEqual(store.state, shown)

        // What the board does show is still taken.
        session.state = .working
        await store.send(.loaded(items: items, sessions: [session]))
        XCTAssertEqual(store.state.sessions.first?.state, .working)
    }

    func testAnUnsavedEditIsThereWhenTheTasksPageOpensAgainAfterAnother() async throws {
        let board = try await board()
        let store = store(board)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.cardTapped(board.task.id))
        await store.send(\.destination.item.editTapped)
        await store.send(\.destination.item.binding.draft, ItemFeature.Draft(title: "login", spec: "# Steps"))
        // As a link to another item opens its page over this one.
        await store.send(.cardTapped(board.subtask.id))
        XCTAssertNil(store.state.destination?.item?.draft)

        await store.send(.cardTapped(board.task.id))
        XCTAssertEqual(store.state.destination?.item?.draft?.spec, "# Steps")

        // Discarded, it isn't there the next time.
        await store.send(\.destination.item.cancelEditTapped)
        await store.send(.destination(.presented(.item(.destination(.presented(.alert(.discard)))))))
        await store.send(.cardTapped(board.subtask.id))
        await store.send(.cardTapped(board.task.id))
        XCTAssertEqual(store.state.destination?.item?.item.id, board.task.id)
        XCTAssertNil(store.state.destination?.item?.draft)
    }

    func testUndoTakesNothingBackOnceEverythingMadeHereIsUndone() async throws {
        let board = try await board()
        let store = TestStore(initialState: BoardFeature.State(project: board.project)) {
            BoardFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
            $0.continuousClock = ImmediateClock()
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.loaded)
        let newest = try await board.daemon.client.boardLog(board.project.id, nil, 1).first?.id
        await store.send(.logStarted(newest: newest))

        await store.send(.dropped(board.task.id, on: .inProgress))
        await store.receive(\.moved)
        await store.receive(\.offerUndo)
        await store.skipReceivedActions(strict: false)
        await store.send(.undoTapped)
        await store.receive(\.undone)
        await store.skipReceivedActions(strict: false)
        let undone = try await board.daemon.client.boardLog(board.project.id, nil, nil)

        // The move was undone, so there is nothing left to take back.
        await store.send(.undoTapped)
        await store.receive(\.undoEnded)
        let after = try await board.daemon.client.boardLog(board.project.id, nil, nil)
        XCTAssertEqual(after, undone)
        XCTAssertFalse(store.state.undoing)
    }

    func testTheJournalSaysItIsReadOnlyOnceItIs() async throws {
        let board = try await board()
        let store = TestStore(initialState: BoardLogFeature.State(project: board.project, items: [])) {
            BoardLogFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
        }
        store.exhaustivity = .off
        XCTAssertFalse(store.state.loaded)

        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertTrue(store.state.loaded)
        XCTAssertFalse(store.state.unread)
    }

    func testAJournalThatCannotBeReadStopsWaitingForIt() async throws {
        let board = try await board()
        let store = TestStore(initialState: BoardLogFeature.State(project: board.project, items: [])) {
            BoardLogFeature()
        } withDependencies: {
            $0.daemon = board.daemon.client
            $0.daemon.boardLog = { _, _, _ in throw DaemonError.disconnected }
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.failed)
        XCTAssertFalse(store.state.loaded)
        XCTAssertTrue(store.state.unread)
        XCTAssertNotNil(store.state.alert)
    }
}
