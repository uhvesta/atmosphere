import ComposableArchitecture
import KhodrBoard
import KhodrDaemonClient
import KhodrProjects
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
        let project = try await daemon.client.createProject("khodr", [Repository(path: "/code/khodr")], "")
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
        await store.receive(\.moved)
        await store.receive(\.loaded)

        XCTAssertEqual(titles(store.state.cards(in: .done)), ["login"])
        XCTAssertEqual(titles(store.state.cards(in: .inReview)), [])
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
        await store.send(\.destination.item.saveTapped)
        await store.receive(\.destination.item.updated)

        XCTAssertNil(store.state.destination?.item?.draft)
        let items = try await board.daemon.client.items(board.project.id)
        XCTAssertEqual(items.first { $0.id == board.subtask.id }?.spec, "# Fields\n- email")
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
        let log = try await board.daemon.client.boardLog(board.project.id)
        XCTAssertEqual(log.first?.reverts, edit.id)
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
}
