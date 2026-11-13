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

    /// A task whose spec was saved twice since it was made, focused.
    private func focusedTask() async throws -> (InMemoryDaemon, Item, TestStoreOf<SpecsFeature>) {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "Be brief.")
        let task = try await daemon.client.createItem(
            project.id, ItemFields(title: "Login", spec: "# Login\n\nSign in."))
        _ = try await daemon.client.updateItem(task.id, ItemUpdate(spec: "# Login\n\nSign in with a passkey."))
        _ = try await daemon.client.updateItem(task.id, ItemUpdate(title: "Sign-in"))
        let store = TestStore(initialState: SpecsFeature.State(projectID: project.id, itemID: task.id)) {
            SpecsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.loaded)
        await store.send(.focusTapped(task.id)) {
            $0.focused = task.id
        }
        return (daemon, task, store)
    }

    func testAFocusedSpecMarksWhatAnEditChangedUntilAnotherIsFocused() async throws {
        let (daemon, task, store) = try await focusedTask()
        _ = try await daemon.client.updateItem(task.id, ItemUpdate(spec: "# Login\n\nSign in with a key."))
        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.changed, [3])

        // Read again unchanged, it keeps its marks.
        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.changed, [3])

        let project = try XCTUnwrap(store.state.pages.first { $0.kind == .project })
        await store.send(.focusTapped(project.id)) {
            $0.focused = project.id
            $0.changed = []
        }
        await store.send(.focusTapped(nil)) {
            $0.focused = nil
        }
    }

    func testASpecsVersionsComeFromTheBoardAndOneIsRestoredAsANewChange() async throws {
        let (daemon, task, store) = try await focusedTask()
        await store.send(.versionsToggled)
        await store.receive(\.versionsListed)
        let versions = try XCTUnwrap(store.state.versions?.list)
        XCTAssertEqual(
            versions.map(\.text), ["# Login\n\nSign in with a passkey.", "# Login\n\nSign in."],
            "Newest first, and only the changes to the spec")

        let first = try XCTUnwrap(versions.last)
        await store.send(.versionPicked(first.id))
        await store.send(.compareToggled)
        await store.receive(\.compared)
        let comparison = try XCTUnwrap(store.state.versions?.comparison)
        XCTAssertEqual(comparison.filter { $0.change == .removed }.map(\.block.text), ["Sign in."])
        XCTAssertEqual(comparison.filter { $0.change == .added }.map(\.block.text), ["Sign in with a passkey."])

        await store.send(.restoreTapped)
        XCTAssertNotNil(store.state.alert, "Restoring asks first")
        await store.send(.alert(.presented(.restore(pageID: task.id, version: first.id))))
        await store.receive(\.restored)
        XCTAssertNil(store.state.versions?.selected)
        XCTAssertEqual(store.state.changed, [3])
        let board = try await daemon.client.items(task.projectID)
        XCTAssertEqual(board.first { $0.id == task.id }?.spec, "# Login\n\nSign in.")
        await store.receive(\.versionsListed)
        XCTAssertEqual(store.state.versions?.list?.count, 3, "The restore is a version of its own")
    }

    func testProjectInstructionsHaveNoVersions() async throws {
        let (_, _, store) = try await focusedTask()
        let project = try XCTUnwrap(store.state.pages.first { $0.kind == .project })
        await store.send(.focusTapped(project.id))
        await store.send(.versionsToggled)
        await store.receive(\.versionsListed)
        XCTAssertEqual(store.state.versions?.list, [])
    }

    func testVersionsThatCantBeReadShowAnAlert() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "Login", spec: "Sign in."))
        var client = daemon.client
        client.boardLog = { _ in throw DaemonError.unexpected("no journal") }
        let store = TestStore(initialState: SpecsFeature.State(projectID: project.id, itemID: task.id)) {
            SpecsFeature()
        } withDependencies: {
            $0.daemon = client
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.loaded)
        await store.send(.focusTapped(task.id))
        await store.send(.versionsToggled)
        await store.receive(\.versionsFailed) {
            $0.versions = nil
        }
        XCTAssertNotNil(store.state.alert)
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
