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
        XCTAssertTrue(store.state.loading)
        await store.receive(\.loaded)
        XCTAssertFalse(store.state.loading)
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
        XCTAssertNotNil(store.state.failure)
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

    func testAFirstListingThatFailsShowsNoListUntilOneIsRead() async throws {
        let daemon = daemon()
        let client = daemon.client
        let fails = LockIsolated(true)
        let store = TestStore(initialState: SkillsFeature.State()) {
            SkillsFeature()
        } withDependencies: {
            $0.daemon = client
            $0.daemon.skills = {
                if fails.value { throw DaemonError.disconnected }
                return try await client.skills()
            }
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.failed)
        XCTAssertNil(store.state.list, "a list that couldn't be read isn't an empty one")
        XCTAssertNotNil(store.state.failure)
        XCTAssertFalse(store.state.loading)

        fails.setValue(false)
        await store.send(.refreshTapped)
        await store.receive(\.loaded)
        XCTAssertNil(store.state.failure)
        XCTAssertEqual(store.state.list?.skills.count, 2)
    }
}
