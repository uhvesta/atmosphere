import ComposableArchitecture
import HumataDaemonClient
import HumataSession
import XCTest

@MainActor
final class ProposalsFeatureTests: XCTestCase {
    private func planning() async throws -> (InMemoryDaemon, Project, Session) {
        let daemon = InMemoryDaemon { _ in
            [
                .propose(title: "Login form", spec: "Email and password."),
                .propose(title: "Dark mode", spec: "Follow the system."),
                .propose(title: "Confetti", spec: "On every save."),
                .say("Three tasks."),
            ]
        }
        let project = try await daemon.client.createProject("demo", [Repository(path: "/code/demo")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, projectID: project.id, planning: true))
        try await daemon.client.send(session.id, "plan it", .enqueue)
        return (daemon, project, session)
    }

    func testTheChatListsWhatTheAgentProposedOnceItHasProposedIt() async throws {
        let (daemon, _, session) = try await planning()
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        let watching = await store.send(.task)
        await store.receive(\.proposals.listed, timeout: .seconds(1))
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(store.state.proposals.proposals.map(\.title), ["Login form", "Dark mode", "Confetti"])
        await watching.cancel()
    }

    func testApplyingAsEditedPutsTheEditOnTheBoardAndDecliningPutsNothing() async throws {
        let (daemon, project, session) = try await planning()
        let proposals = try await daemon.client.proposals(session.id)
        let store = TestStore(initialState: ProposalsFeature.State(sessionID: session.id)) {
            ProposalsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.refresh)
        await store.receive(\.listed)
        await store.send(.applyTapped(proposals[0].id))
        await store.receive(\.decided)
        await store.send(.editTapped(proposals[1].id))
        let edited = ProposalsFeature.Editing(id: proposals[1].id, title: "Dark theme", spec: "Always dark.")
        await store.send(\.binding.editing, edited)
        await store.send(.editApplied)
        await store.receive(\.decided)
        await store.send(.declineTapped(proposals[2].id))
        await store.receive(\.decided)

        let board = try await daemon.client.items(project.id)
        XCTAssertEqual(board.map(\.title), ["Login form", "Dark theme"])
        XCTAssertEqual(board.map(\.spec), ["Email and password.", "Always dark."])
        let states = store.state.proposals.map(\.state)
        XCTAssertEqual(states.last, .declined)
        XCTAssertEqual(states.filter { $0 != .declined && $0 != .pending }.count, 2)
    }

    func testStartingWorkOnAnAppliedTaskOpensASessionOnIt() async throws {
        let (daemon, _, session) = try await planning()
        let proposals = try await daemon.client.proposals(session.id)
        let item = try await daemon.client.applyProposal(session.id, proposals[0].id, nil, nil)
        let store = TestStore(initialState: ProposalsFeature.State(sessionID: session.id)) {
            ProposalsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.startWorkTapped(itemID: item.id))
        await store.receive(\.delegate.openSession)
        let sessions = try await daemon.client.sessions(nil)
        let work = try XCTUnwrap(sessions.first { $0.parentID == session.id })
        XCTAssertEqual(work.itemID, item.id)
        XCTAssertFalse(work.planning)
    }

    func testAProposalDecidedElsewhereShowsAsItStands() async throws {
        let (daemon, _, session) = try await planning()
        let proposals = try await daemon.client.proposals(session.id)
        let store = TestStore(initialState: ProposalsFeature.State(sessionID: session.id)) {
            ProposalsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.listed)
        try await daemon.client.declineProposal(session.id, proposals[0].id)

        await store.send(.applyTapped(proposals[0].id))
        await store.receive(\.failed)
        await store.receive(\.listed)
        XCTAssertEqual(store.state.proposals.first?.state, .declined)
        XCTAssertNotNil(store.state.alert)
    }

    func testAnEditThatCouldntBeAppliedStaysOpenUntilItIs() async throws {
        let (daemon, project, session) = try await planning()
        let proposals = try await daemon.client.proposals(session.id)
        let reachable = LockIsolated(false)
        let store = TestStore(initialState: ProposalsFeature.State(sessionID: session.id)) {
            ProposalsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.applyProposal = { session, id, title, spec in
                guard reachable.value else { throw DaemonError.disconnected }
                return try await daemon.client.applyProposal(session, id, title, spec)
            }
        }
        store.exhaustivity = .off

        await store.send(.refresh)
        await store.receive(\.listed)
        await store.send(.editTapped(proposals[0].id))
        let edited = ProposalsFeature.Editing(id: proposals[0].id, title: "Sign-in form", spec: "With a passkey.")
        await store.send(\.binding.editing, edited)
        await store.send(.editApplied)
        await store.receive(\.failed)
        await store.receive(\.listed)
        XCTAssertEqual(store.state.editing, edited)

        reachable.setValue(true)
        await store.send(.editApplied)
        await store.receive(\.decided)
        XCTAssertNil(store.state.editing)
        let board = try await daemon.client.items(project.id)
        XCTAssertEqual(board.map(\.title), ["Sign-in form"])
    }
}
