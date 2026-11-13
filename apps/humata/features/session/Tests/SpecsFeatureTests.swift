import ComposableArchitecture
import HumataDaemonClient
import XCTest

@testable import HumataSession

@MainActor
final class SpecsFeatureTests: XCTestCase {
    func testASubtasksChatReadsTheProjectThenItsTaskThenItself() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "Be brief.")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "Login", spec: "Sign in."))
        let subtask = try await daemon.client.createItem(
            project.id, ItemFields(parentID: task.id, title: "Form", spec: "Two fields."))
        let store = TestStore(initialState: SpecsFeature.State(projectID: project.id, itemID: subtask.id)) {
            SpecsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.pages.map(\.kind), [.project, .task, .subtask])
        XCTAssertEqual(store.state.pages.map(\.text), ["Be brief.", "Sign in.", "Two fields."])
    }

    func testAnEditIsSavedToTheBoard() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "Login", spec: "Sign in."))
        let store = TestStore(initialState: SpecsFeature.State(projectID: project.id, itemID: task.id)) {
            SpecsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.refresh)
        await store.receive(\.loaded)
        await store.send(.editTapped(task.id))
        await store.send(.draftChanged("Sign in with a passkey."))
        await store.send(.saveTapped)
        await store.receive(\.saved)
        XCTAssertNil(store.state.draft)
        XCTAssertEqual(store.state.pages[id: task.id]?.text, "Sign in with a passkey.")
        let board = try await daemon.client.items(project.id)
        XCTAssertEqual(board.first { $0.id == task.id }?.spec, "Sign in with a passkey.")
    }

    func testAChatOnTheProjectItselfReadsOnlyItsInstructions() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "Be brief.")
        let store = TestStore(initialState: SpecsFeature.State(projectID: project.id, itemID: nil)) {
            SpecsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.pages.map(\.kind), [.project])
    }
}
