import ComposableArchitecture
import HumataDaemonClient
import HumataProjects
import XCTest

@MainActor
final class SkillsFeatureTests: XCTestCase {
    private let mine = FoundSkill(
        name: "release", path: "/home/.claude/skills/release/SKILL.md", origin: .claudeUser, hash: "a1")
    private let theirs = FoundSkill(
        name: "release", path: "/code/.github/skills/release/SKILL.md", origin: .githubRepository, projectID: "p",
        repository: "code", hash: "b2")

    private func daemon(clashing: Bool = true) -> InMemoryDaemon {
        let daemon = InMemoryDaemon()
        daemon.setSkills(
            SkillList(
                skills: [mine, theirs],
                clashes: clashing ? [SkillClash(name: "release", candidates: [mine, theirs])] : []))
        return daemon
    }

    func testListingShowsTheClashesUnpicked() async {
        let store = TestStore(initialState: SkillsFeature.State()) {
            SkillsFeature()
        } withDependencies: {
            $0.daemon = daemon().client
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.list?.skills.count, 2)
        XCTAssertEqual(store.state.unpicked, 1)
    }

    func testPickingOneHoldsAndLetsASessionStart() async throws {
        let daemon = daemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let store = TestStore(initialState: SkillsFeature.State()) {
            SkillsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)
        let clash = try XCTUnwrap(store.state.list?.clashes.first)

        await store.send(.picked(clash.id, winner: "b2"))
        await store.receive(\.pickSaved)
        XCTAssertEqual(store.state.unpicked, 0)
        XCTAssertTrue(store.state.saving.isEmpty)

        await store.send(.refreshTapped)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.list?.clashes.first?.winner, "b2")

        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, name: "After the pick", projectID: project.id))
        XCTAssertEqual(session.projectID, project.id)
    }

    func testPickingAFileNotInTheClashFailsAndListsAgain() async throws {
        let store = TestStore(initialState: SkillsFeature.State()) {
            SkillsFeature()
        } withDependencies: {
            $0.daemon = daemon().client
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)
        let clash = try XCTUnwrap(store.state.list?.clashes.first)

        await store.send(.picked(clash.id, winner: "zz"))
        await store.receive(\.pickFailed)
        await store.receive(\.loaded)
        XCTAssertNil(store.state.list?.clashes.first?.winner)
        XCTAssertEqual(store.state.unpicked, 1)
    }

    func testANewSessionWaitsForThePickAndSaysWhere() async throws {
        let daemon = daemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let store = TestStore(initialState: NewSessionFeature.State(project: project)) {
            NewSessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        await store.send(\.binding.backend, .scripted)
        await store.send(.startTapped)
        await store.receive(\.skillsClash)
        XCTAssertNotNil(store.state.skillClash)
        XCTAssertTrue(daemon.sessions.isEmpty)

        try await daemon.client.pickSkill("release", ["a1", "b2"], "a1")
        await store.send(.startTapped)
        await store.receive(\.delegate.started)
        XCTAssertNil(store.state.skillClash)
    }
}
