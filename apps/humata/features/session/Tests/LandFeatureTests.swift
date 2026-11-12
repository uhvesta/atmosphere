import ComposableArchitecture
import HumataDaemonClient
import HumataSession
import XCTest

@MainActor
final class LandFeatureTests: XCTestCase {
    private func session(_ daemon: InMemoryDaemon) async throws -> Session {
        let project = try await daemon.client.createProject("Humata", [Repository(path: "/code/humata")], "")
        return try await daemon.client.start(
            StartRequest(backend: .scripted, name: "Fix the login form!", projectID: project.id))
    }

    func testTheBranchIsNamedAfterTheTitle() {
        XCTAssertEqual(LandFeature.State.branch(for: "Fix the login form!"), "fix-the-login-form")
        XCTAssertEqual(LandFeature.State.branch(for: "  !! "), "humata-work")
    }

    func testLandingOpensAPullRequestAndLandingAgainKeepsIt() async throws {
        let daemon = InMemoryDaemon()
        let session = try await session(daemon)
        let store = TestStore(initialState: LandFeature.State(sessionID: session.id, title: session.name)) {
            LandFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.landTapped)
        await store.receive(\.landed)
        let landed = try XCTUnwrap(store.state.landed)
        XCTAssertEqual(landed.repositories.map(\.branch), ["fix-the-login-form"])
        XCTAssertTrue(landed.repositories.allSatisfy(\.pushed))
        XCTAssertEqual(landed.pullRequests.count, 1)
        XCTAssertFalse(store.state.canLand)

        let again = try await daemon.client.land(
            session.id, LandRequest(branch: "fix-the-login-form", message: "Again", openPullRequests: true))
        XCTAssertEqual(again.pullRequests.map(\.number), landed.pullRequests.map(\.number))
    }

    func testABranchGitWontTakeIsRefusedAndCanBeFixed() async throws {
        let daemon = InMemoryDaemon()
        let session = try await session(daemon)
        let store = TestStore(initialState: LandFeature.State(sessionID: session.id, title: session.name)) {
            LandFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.binding.branch, "fix login")
        await store.send(.landTapped)
        await store.receive(\.failed)
        XCTAssertNotNil(store.state.error)
        XCTAssertNil(store.state.landed)

        await store.send(\.binding.branch, "fix-login")
        await store.send(.landTapped)
        await store.receive(\.landed)
        XCTAssertNil(store.state.error)
    }

    func testOnlyAProjectsSessionShips() async throws {
        let daemon = InMemoryDaemon()
        let chat = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        XCTAssertFalse(SessionFeature.State(session: chat).ships)
        let task = try await session(daemon)
        XCTAssertTrue(SessionFeature.State(session: task).ships)
    }
}
