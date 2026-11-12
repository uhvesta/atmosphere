import ComposableArchitecture
import HumataDaemonClient
import HumataProjects
import XCTest

@MainActor
final class ApprovalsTests: XCTestCase {
    private let asked = Date(timeIntervalSince1970: 1_794_000_000)

    private func approval(_ id: String, scope: ApprovalScope = .board) -> AdminApproval {
        AdminApproval(
            id: id, client: "humata mcp", parent: "claude", scope: scope, summary: "Add task \(id)", asked: asked)
    }

    private func sidebar(_ daemon: InMemoryDaemon) -> TestStoreOf<ProjectsFeature> {
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(asked)
            $0.daemon = daemon.client
            $0.notifications = .previewValue
            $0.appActivity = AppActivity(
                isActive: { true }, activations: { AsyncStream { $0.finish() } }, setBadge: { _ in })
        }
        store.exhaustivity = .off
        return store
    }

    func testAdminActionsWaitInTheInboxAndBlockOnYou() async {
        let daemon = InMemoryDaemon()
        daemon.askApproval(approval("a1"))
        daemon.askApproval(approval("a2", scope: .destructive))
        let store = sidebar(daemon)

        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.approvals.map(\.id), ["a1", "a2"])
        XCTAssertEqual(store.state.blockedCount, 2)
    }

    func testDenyingOneLeavesTheOthersWaiting() async throws {
        let daemon = InMemoryDaemon()
        daemon.askApproval(approval("a1"))
        daemon.askApproval(approval("a2"))
        let store = sidebar(daemon)
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.approvalsDecided(["a1"], approve: false))
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.approvals.map(\.id), ["a2"])
        let kept = try await daemon.client.approvals()
        XCTAssertEqual(kept.approvals.first { $0.id == "a1" }?.state, .denied)
        XCTAssertTrue(kept.grants.isEmpty)
    }

    func testApprovingAllWithAGrantClearsTheInboxAndGrantsTheirScopes() async throws {
        let daemon = InMemoryDaemon()
        daemon.askApproval(approval("a1"))
        daemon.askApproval(approval("a2", scope: .proposals))
        let store = sidebar(daemon)
        await store.send(.refresh)
        await store.receive(\.loaded)

        let grant = ApprovalGrantAsked(scopes: [.proposals, .board], duration: .seconds(3600))
        await store.send(.approvalsDecided(["a1", "a2"], approve: true, grant: grant))
        await store.receive(\.loaded)
        XCTAssertTrue(store.state.approvals.isEmpty)
        XCTAssertEqual(store.state.blockedCount, 0)
        let kept = try await daemon.client.approvals()
        XCTAssertEqual(kept.approvals.map(\.state), [.approved, .approved])
        XCTAssertEqual(kept.grants.map(\.scopes), [[.proposals, .board]])
        XCTAssertEqual(kept.grants.first?.expires, daemon.start.addingTimeInterval(3600))
    }

    func testAFailedDecisionSaysWhyAndShowsTheApprovalAgain() async {
        let daemon = InMemoryDaemon()
        daemon.askApproval(approval("a1"))
        let store = sidebar(daemon)
        await store.send(.refresh)
        await store.receive(\.loaded)
        store.dependencies.daemon.decideApprovals = { _, _, _, _ in
            throw DaemonError.refused(.appOnly, message: "only the app")
        }

        await store.send(.approvalsDecided(["a1"], approve: true))
        await store.receive(\.answerFailed)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.approvals.map(\.id), ["a1"])
    }

    func testRevokingAGrantTakesItOutOfForce() async throws {
        let daemon = InMemoryDaemon()
        let grant = daemon.grant([.board], until: daemon.start.addingTimeInterval(900))
        let clock = TestClock()
        let store = TestStore(initialState: ApprovalsFeature.State()) {
            ApprovalsFeature()
        } withDependencies: {
            $0.date = .constant(asked)
            $0.daemon = daemon.client
            $0.continuousClock = clock
        }
        store.exhaustivity = .off

        let task = await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.approvals.grants.map(\.id), [grant.id])

        await store.send(.revokeTapped(grant.id)) {
            $0.approvals.grants = []
        }
        await store.receive(\.loaded)
        let kept = try await daemon.client.approvals()
        XCTAssertTrue(kept.grants.isEmpty)
        await task.cancel()
    }

    func testRevokingAGrantAlreadyGoneSaysSo() async {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: ApprovalsFeature.State()) {
            ApprovalsFeature()
        } withDependencies: {
            $0.date = .constant(asked)
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.revokeTapped("g9"))
        await store.receive(\.revokeFailed)
        XCTAssertNotNil(store.state.failure)
    }
}
