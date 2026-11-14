import ComposableArchitecture
import HumataDaemonClient
import HumataProjects
import XCTest

/// Plan With an Agent with a read-only copy of the code: off until the
/// project's default or the person says so.
@MainActor
final class PlanWithCodeTests: XCTestCase {
    private func plan(in daemon: InMemoryDaemon, withCode: Bool?) async throws -> Session {
        let projects = try await daemon.client.projects()
        let project = try XCTUnwrap(projects.first)
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
        if let withCode {
            await store.send(\.destination.newSession.binding.withCode, withCode)
        }
        await store.send(.destination(.presented(.newSession(.startTapped))))
        await store.receive(\.destination.newSession.delegate.started)
        return try XCTUnwrap(daemon.sessions.last)
    }

    func testPlanningStartsWithNoCodeUnlessAskedFor() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.createProject("code", [Repository(path: "/code")], "")

        let plain = try await plan(in: daemon, withCode: nil)
        XCTAssertTrue(plain.planning)
        XCTAssertFalse(plain.plansWithCode)

        let withCode = try await plan(in: daemon, withCode: true)
        XCTAssertTrue(withCode.plansWithCode)
        XCTAssertEqual(withCode.access, .plan)
    }

    func testTheProjectsDefaultStartsWithTheCodeAndCanBeTurnedOff() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        _ = try await daemon.client.updateProject(project.id, ProjectUpdate(planWithCode: true))

        let byDefault = try await plan(in: daemon, withCode: nil)
        XCTAssertTrue(byDefault.plansWithCode)

        let turnedOff = try await plan(in: daemon, withCode: false)
        XCTAssertFalse(turnedOff.plansWithCode)
    }

    func testOnlyAPlanningSessionIsOfferedTheCode() async throws {
        let daemon = InMemoryDaemon()
        var project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        project.planWithCode = true

        var working = NewSessionFeature.State(project: project)
        working.withCode = true
        XCTAssertFalse(working.offersWithCode)
        XCTAssertFalse(working.request.withCode)

        let planning = NewSessionFeature.State(project: project, planning: true)
        XCTAssertTrue(planning.offersWithCode)
        XCTAssertTrue(planning.request.withCode)
    }

    func testProjectSettingsSaveThePlanningDefault() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let store = TestStore(initialState: ProjectSettingsFeature.State(project: project)) {
            ProjectSettingsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(\.binding.planWithCode, true)
        await store.send(.saveTapped)
        await store.receive(\.delegate.saved)

        let saved = try await daemon.client.projects()
        XCTAssertEqual(saved.first?.planWithCode, true)
    }
}
