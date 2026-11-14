import ComposableArchitecture
import HumataBoard
import HumataDaemonClient
import XCTest

/// The board's keyboard: arrows move the selection, Return opens the
/// selected card, Space asks for its status.
@MainActor
final class BoardKeyboardTests: XCTestCase {
    /// A board with "login" over "copy" in To Do, by priority, and "form"
    /// in Done; the columns between are empty.
    private func store() async throws -> (TestStoreOf<BoardFeature>, login: Item, copy: Item, form: Item) {
        let daemon = InMemoryDaemon(script: { _ in [] })
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let login = try await daemon.client.createItem(project.id, ItemFields(title: "login", priority: .high))
        let copy = try await daemon.client.createItem(project.id, ItemFields(title: "copy"))
        let form = try await daemon.client.createItem(project.id, ItemFields(title: "form", status: .done))
        let store = TestStore(initialState: BoardFeature.State(project: project)) {
            BoardFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.loaded)
        return (store, login, copy, form)
    }

    func testAnArrowWithNothingSelectedSelectsTheFirstCard() async throws {
        let (store, login, _, _) = try await store()
        await store.send(.selectionMoved(.down))
        XCTAssertEqual(store.state.selected, login.id)
    }

    func testUpAndDownMoveWithinAColumnAndStopAtItsEnds() async throws {
        let (store, login, copy, _) = try await store()
        await store.send(.cardTapped(login.id))
        await store.send(.destination(.dismiss))
        await store.send(.selectionMoved(.up))
        XCTAssertEqual(store.state.selected, login.id)
        await store.send(.selectionMoved(.down))
        XCTAssertEqual(store.state.selected, copy.id)
        await store.send(.selectionMoved(.down))
        XCTAssertEqual(store.state.selected, copy.id)
    }

    func testLeftAndRightSkipEmptyColumnsAndStopAtTheEdges() async throws {
        let (store, _, copy, form) = try await store()
        await store.send(.cardTapped(copy.id))
        await store.send(.destination(.dismiss))
        await store.send(.selectionMoved(.left))
        XCTAssertEqual(store.state.selected, copy.id)
        // Done has one card, so the second row lands on it.
        await store.send(.selectionMoved(.right))
        XCTAssertEqual(store.state.selected, form.id)
        await store.send(.selectionMoved(.right))
        XCTAssertEqual(store.state.selected, form.id)
    }

    func testReturnOpensTheSelectedCard() async throws {
        let (store, login, copy, _) = try await store()
        await store.send(.openSelectedTapped)
        XCTAssertNil(store.state.destination)

        await store.send(.selectionMoved(.down))
        await store.send(.selectionMoved(.down))
        await store.send(.openSelectedTapped)
        await store.receive(\.cardTapped)
        XCTAssertEqual(store.state.destination?.item?.item.id, copy.id)
        XCTAssertNotEqual(store.state.destination?.item?.item.id, login.id)
    }

    func testSpaceAsksForTheStatusAndChangesNothingUntilOneIsPicked() async throws {
        let (store, login, _, _) = try await store()
        await store.send(.statusPickerTapped)
        XCTAssertNil(store.state.destination)

        await store.send(.selectionMoved(.down))
        await store.send(.statusPickerTapped)
        XCTAssertNotNil(store.state.destination?.statusPicker)
        XCTAssertEqual(store.state.items[id: login.id]?.status, .todo)

        await store.send(.destination(.presented(.statusPicker(.set(login.id, .inProgress)))))
        await store.receive(\.dropped)
        await store.receive(\.moved)
        XCTAssertEqual(store.state.items[id: login.id]?.status, .inProgress)
        XCTAssertNil(store.state.destination?.statusPicker)
    }
}
