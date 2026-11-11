import ComposableArchitecture
import FlowKit
import KhodrAppFeature
import KhodrDaemonClient
import KhodrSession
import SwiftUI
import XCTest

/// Khodr's window over an in-memory daemon. The Mac flow drives the store,
/// since it can't click.
@MainActor
final class MacFlowSnapshotTests: XCTestCase {
    private struct TimedOut: Error {}

    /// An agent edits a file in a project: the edit waits for the person,
    /// is approved, and a command then waits until the session allows
    /// everything.
    func testAnEditThroughAnApproval() async throws {
        let daemon = InMemoryDaemon { _ in
            [
                .think("The notes go in the repository."),
                .write(path: "/code/khodr/NOTES.md", contents: "Ship M1."),
                .run(command: "git status", output: "nothing to commit, working tree clean"),
                .say("Wrote the notes and checked the tree is clean."),
            ]
        }
        let project = try await daemon.client.createProject("khodr", [Repository(path: "/code/khodr")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, name: "Release notes", projectID: project.id))
        // The sidebar is read again only when the flow asks, so a refresh
        // never lands between a step settling and its capture.
        let store = Store(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemonService = .previewValue
            $0.continuousClock = TestClock()
        }
        let flow = FlowRecorder("edit", AppView(store: store))

        try await until { store.sidebar.sessions.count == 1 && store.daemon == .running }
        await flow.step("sidebar")

        store.send(.sidebar(.selected(.session(session.id))))
        try await until { store.session != nil }
        store.send(.session(.presented(.binding(.set(\.draft, "Write the release notes for M1")))))
        store.send(.session(.presented(.sendTapped(.enqueue))))
        try await until { store.session?.isWaiting == true }
        try await sidebarShows(.waitingForApproval, store, session)
        await flow.step("waiting")

        guard case .tool(let write)? = store.session?.items.last, let request = write.permission?.requestID else {
            return XCTFail("the write isn't waiting")
        }
        store.send(.session(.presented(.answerPermission(requestID: request, approved: true))))
        try await until { store.session?.isWaiting == true && daemon.files(of: session.id).isEmpty == false }
        try await sidebarShows(.waitingForApproval, store, session)
        await flow.step("approved")

        store.send(.session(.presented(.approvalModePicked(.allowEverything))))
        try await until { store.session?.isWorking == false }
        try await sidebarShows(.idle, store, session)
        await flow.step("done")
        flow.finish()
    }

    /// A session is forked before its second turn: the fork shows under it
    /// in the sidebar with the chat up to there, and goes its own way.
    func testAForkBeforeATurn() async throws {
        let daemon = InMemoryDaemon { text in
            [.write(path: "/\(text).md", contents: text), .say("Wrote \(text).md.")]
        }
        let project = try await daemon.client.createProject("khodr", [Repository(path: "/code/khodr")], "")
        let session = try await daemon.client.start(
            StartRequest(
                backend: .scripted, name: "Release notes", approvalMode: .allowEverything, projectID: project.id))
        try await daemon.client.send(session.id, "notes", .enqueue)
        try await daemon.client.send(session.id, "changelog", .enqueue)
        let store = Store(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemonService = .previewValue
            $0.continuousClock = TestClock()
        }
        let flow = FlowRecorder("fork", AppView(store: store))

        try await until { store.sidebar.sessions.count == 1 && store.daemon == .running }
        store.send(.sidebar(.selected(.session(session.id))))
        try await until { store.session?.items.count == 6 }
        await flow.step("session")

        let second = try XCTUnwrap(daemon.history(session.id).first { $0.kind == .userMessage("changelog") })
        store.send(.session(.presented(.forkTapped(before: second.id, chatOnly: false))))
        try await until { store.sidebar.sessions.count == 2 && store.session?.session.id != session.id }
        let fork = try XCTUnwrap(store.session?.session)
        try await until { store.session?.items.count == 3 }
        XCTAssertEqual(daemon.files(of: fork.id), ["/notes.md": "notes"])
        await flow.step("forked")

        store.send(.session(.presented(.binding(.set(\.draft, "readme")))))
        store.send(.session(.presented(.sendTapped(.enqueue))))
        try await until { store.session?.items.count == 6 }
        XCTAssertEqual(daemon.files(of: fork.id), ["/notes.md": "notes", "/readme.md": "readme"])
        XCTAssertEqual(daemon.files(of: session.id), ["/notes.md": "notes", "/changelog.md": "changelog"])
        await flow.step("went its own way")
        flow.finish()
    }

    /// A session on a subtask moves it and its task across the board, to
    /// Waiting on you when it asks for an approval, and shows in the inbox.
    func testTheBoard() async throws {
        let daemon = InMemoryDaemon { _ in
            [
                .write(path: "/code/khodr/LoginForm.swift", contents: "struct LoginForm {}"),
                .say("Added the form."),
            ]
        }
        let project = try await daemon.client.createProject("khodr", [Repository(path: "/code/khodr")], "")
        let spec = """
            People sign in with **email and a password**.

            - Show an error under the field that's wrong
            - Keep them signed in for 30 days
            """
        let task = try await daemon.client.createItem(
            project.id, ItemFields(title: "Sign in", spec: spec, priority: .high))
        let form = try await daemon.client.createItem(
            project.id, ItemFields(parentID: task.id, title: "Login form", spec: "The form itself."))
        _ = try await daemon.client.createItem(project.id, ItemFields(parentID: task.id, title: "Error copy"))
        _ = try await daemon.client.createItem(project.id, ItemFields(title: "Dark mode", priority: .low))
        let store = Store(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemonService = .previewValue
            $0.continuousClock = TestClock()
        }
        let flow = FlowRecorder("board", AppView(store: store))

        try await until { store.daemon == .running && store.sidebar.projects.count == 1 }
        store.send(.sidebar(.selected(.board(project.id))))
        try await until { store.board?.items.count == 4 }
        await flow.step("board")

        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, name: "Login form", itemID: form.id))
        try await daemon.client.send(session.id, "Build the form", .enqueue)
        try await until { daemon.sessions.first?.state == .waitingForApproval }
        store.send(.board(.presented(.refresh)))
        try await until { store.board?.items[id: task.id]?.status == .waitingOnYou }
        await flow.step("waiting on you")

        store.send(.board(.presented(.cardTapped(task.id))))
        try await until { store.board?.destination?.item != nil }
        await flow.step("task")

        store.send(.board(.presented(.destination(.dismiss))))
        store.send(.sidebar(.refresh))
        try await until { store.sidebar.inbox.count == 1 }
        store.send(.sidebar(.selected(.inbox)))
        try await until { store.sidebar.selection == .inbox }
        await flow.step("inbox")
        flow.finish()
    }

    private func sidebarShows(
        _ state: SessionState, _ store: StoreOf<AppFeature>, _ session: Session
    ) async throws {
        store.send(.sidebar(.refresh))
        try await until { store.sidebar.sessions[id: session.id]?.state == state }
    }

    private func until(_ condition: () -> Bool) async throws {
        for _ in 0..<500 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        throw TimedOut()
    }
}
