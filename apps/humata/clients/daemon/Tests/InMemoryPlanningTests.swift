import HumataDaemonClient
import XCTest

final class InMemoryPlanningTests: XCTestCase {
    private func planning() async throws -> (InMemoryDaemon, Project, Session) {
        let daemon = InMemoryDaemon { _ in
            [
                .propose(title: "Login form", spec: "Email and password."),
                .propose(title: "Dark mode", spec: "Follow the system."),
                .say("Two tasks."),
            ]
        }
        let project = try await daemon.client.createProject(
            "demo", [Repository(name: "app", path: "/repos/app")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, projectID: project.id, planning: true))
        try await daemon.client.send(session.id, "plan it", .enqueue)
        return (daemon, project, session)
    }

    func testNothingIsOnTheBoardUntilAProposalIsApplied() async throws {
        let (daemon, project, session) = try await planning()
        let proposals = try await daemon.client.proposals(session.id)
        XCTAssertEqual(proposals.map(\.title), ["Login form", "Dark mode"])
        XCTAssertEqual(proposals.map(\.state), [.pending, .pending])
        let empty = try await daemon.client.items(project.id)
        XCTAssertEqual(empty, [])

        let item = try await daemon.client.applyProposal(session.id, proposals[0].id, "Sign in", nil)
        try await daemon.client.declineProposal(session.id, proposals[1].id)

        let board = try await daemon.client.items(project.id)
        XCTAssertEqual(board.map(\.title), ["Sign in"])
        XCTAssertEqual(board.first?.spec, "Email and password.")
        let decided = try await daemon.client.proposals(session.id)
        XCTAssertEqual(decided.map(\.state), [.applied(itemID: item.id), .declined])
    }

    func testAProposalIsDecidedOnce() async throws {
        let (daemon, _, session) = try await planning()
        let proposals = try await daemon.client.proposals(session.id)
        let proposal = try XCTUnwrap(proposals.first)
        try await daemon.client.declineProposal(session.id, proposal.id)
        do {
            _ = try await daemon.client.applyProposal(session.id, proposal.id, nil, nil)
            XCTFail("applied a declined proposal")
        } catch DaemonError.refused(let code, _) {
            XCTAssertEqual(code, .unknownProposal)
        }
    }

    func testWorkOnAnAppliedTaskGetsACopyOfTheCode() async throws {
        let (daemon, _, session) = try await planning()
        XCTAssertTrue(session.planning)
        let proposals = try await daemon.client.proposals(session.id)
        let proposal = try XCTUnwrap(proposals.first)
        let item = try await daemon.client.applyProposal(session.id, proposal.id, nil, nil)

        let work = try await daemon.client.startWork(session.id, item.id)

        XCTAssertEqual(work.parentID, session.id)
        XCTAssertEqual(work.itemID, item.id)
        XCTAssertFalse(work.planning)
        XCTAssertNotEqual(work.workingDirectory, session.workingDirectory)
        let history = try await daemon.client.history(work.id)
        XCTAssertEqual(history.count, daemon.history(session.id).count)
    }

    func testAProjectsSkillsCanBeChanged() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject(
            "demo", [Repository(name: "app", path: "/repos/app")], "")
        let updated = try await daemon.client.updateProject(
            project.id,
            ProjectUpdate(skills: [Skill(name: "Tests", text: "make test"), Skill(name: " ", text: "dropped")]))
        XCTAssertEqual(updated.skills, [Skill(name: "Tests", text: "make test")])
        let listed = try await daemon.client.projects()
        XCTAssertEqual(listed.first?.skills, updated.skills)
    }
}
