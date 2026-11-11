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
        let project = try await daemon.client.createProject("khodr", "/code/khodr", "")
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

        store.send(.sidebar(.sessionSelected(session.id)))
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
        try await until { store.session?.isWaiting == true && daemon.files.isEmpty == false }
        try await sidebarShows(.waitingForApproval, store, session)
        await flow.step("approved")

        store.send(.session(.presented(.approvalModePicked(.allowEverything))))
        try await until { store.session?.isWorking == false }
        try await sidebarShows(.idle, store, session)
        await flow.step("done")
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
