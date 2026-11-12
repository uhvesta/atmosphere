import ComposableArchitecture
import HumataDaemonClient
import HumataProjects
import XCTest

@MainActor
final class PlanningFeatureTests: XCTestCase {
    func testPlanningInAProjectStartsASessionWithNoCopyOfTheCode() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let store = TestStore(initialState: ProjectsFeature.State()) {
            ProjectsFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.continuousClock = ImmediateClock()
        }
        store.exhaustivity = .off
        await store.send(.refresh)
        await store.receive(\.loaded)

        await store.send(.planTapped(projectID: project.id))
        await store.send(\.destination.newSession.binding.backend, .scripted)
        await store.send(.destination(.presented(.newSession(.startTapped))))
        await store.receive(\.destination.newSession.delegate.started)

        let session = try XCTUnwrap(daemon.sessions.first)
        XCTAssertTrue(session.planning)
        XCTAssertEqual(session.projectID, project.id)
        XCTAssertFalse(session.workingDirectory.hasPrefix("/copies/"))
    }

    func testASessionOnATaskNeverPlans() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let item = try await daemon.client.createItem(project.id, ItemFields(title: "Login"))
        let state = NewSessionFeature.State(project: project, item: item, planning: true)
        XCTAssertFalse(state.planning)
    }

    func testProjectSettingsSaveTheSkillsWrittenAndDropEmptyOnes() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "be brief")
        let store = TestStore(initialState: ProjectSettingsFeature.State(project: project)) {
            ProjectSettingsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.addSkillTapped)
        await store.send(.addSkillTapped)
        let rows = store.state.skills.ids
        await store.send(
            \.binding.skills,
            [
                ProjectSettingsFeature.SkillRow(id: rows[0], name: "Tests", text: "make test"),
                ProjectSettingsFeature.SkillRow(id: rows[1], name: "Empty"),
            ])
        await store.send(\.binding.instructions, "be thorough")
        await store.send(.saveTapped)
        await store.receive(\.delegate.saved)

        let saved = try await daemon.client.projects()
        XCTAssertEqual(saved.first?.skills, [Skill(name: "Tests", text: "make test")])
        XCTAssertEqual(saved.first?.instructions, "be thorough")
        XCTAssertEqual(saved.first?.name, "code")
    }
}
