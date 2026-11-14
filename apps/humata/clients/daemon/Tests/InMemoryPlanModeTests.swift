import HumataDaemonClient
import XCTest

/// Plan mode in the in-memory daemon, as the real one has it: a session
/// switched to it, or planning with the code, writes nothing.
final class InMemoryPlanModeTests: XCTestCase {
    private let edit: @Sendable (String) -> [InMemoryDaemon.Step] = { _ in
        [.write(path: "/work/a.txt", contents: "a"), .say("Done.")]
    }

    private func notices(_ events: [SessionEvent]) -> Int {
        events.filter { if case .notice = $0.kind { true } else { false } }.count
    }

    func testASessionInPlanModeWritesNothingEvenAllowedEverything() async throws {
        let daemon = InMemoryDaemon(script: edit)
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", approvalMode: .allowEverything))
        let planning = try await daemon.client.update(session.id, SessionUpdate(access: .plan))
        XCTAssertEqual(planning.access, .plan)

        try await daemon.client.send(session.id, "write it", .enqueue)

        XCTAssertEqual(daemon.files(of: session.id), [:])
        XCTAssertEqual(daemon.sessions.first?.state, .idle)
    }

    func testSwitchingBackLetsItWriteAndEachSwitchIsNoted() async throws {
        let daemon = InMemoryDaemon(script: edit)
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", approvalMode: .allowEverything))
        _ = try await daemon.client.update(session.id, SessionUpdate(access: .plan))
        let full = try await daemon.client.update(session.id, SessionUpdate(access: .full))
        XCTAssertEqual(full.access, .full)

        try await daemon.client.send(session.id, "write it", .enqueue)

        XCTAssertEqual(daemon.files(of: session.id), ["/work/a.txt": "a"])
        XCTAssertEqual(notices(daemon.history(session.id)), 2)
    }

    func testAskingForTheAccessItHasNotesNothing() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        _ = try await daemon.client.update(session.id, SessionUpdate(access: .full))
        XCTAssertEqual(notices(daemon.history(session.id)), 0)
    }

    func testPlanningWithTheCodeReadsACopyAndCantBeSwitched() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject(
            "demo", [Repository(name: "app", path: "/repos/app")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, projectID: project.id, planning: true, withCode: true))
        XCTAssertTrue(session.plansWithCode)
        XCTAssertTrue(session.workingDirectory.hasPrefix("/copies/"))

        let commands = try await daemon.client.chatCommands(session.id)
        XCTAssertFalse(commands.plan)
        do {
            _ = try await daemon.client.update(session.id, SessionUpdate(access: .full))
            XCTFail("switched a planning session with the code out of plan mode")
        } catch DaemonError.refused(let code, _) {
            XCTAssertEqual(code, .invalidRequest)
        }
    }

    func testPlanningWithoutTheCodeGetsNoCopy() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject(
            "demo", [Repository(name: "app", path: "/repos/app")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, projectID: project.id, planning: true))
        XCTAssertFalse(session.plansWithCode)
        XCTAssertFalse(session.workingDirectory.hasPrefix("/copies/"))
    }

    func testASideChatCantBeSwitched() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        let side = try await daemon.client.newChild(session.id, "aside", .side)
        XCTAssertEqual(side.access, .sideChat)
        do {
            _ = try await daemon.client.update(side.id, SessionUpdate(access: .plan))
            XCTFail("switched a side chat")
        } catch DaemonError.refused(let code, _) {
            XCTAssertEqual(code, .invalidRequest)
        }
    }

    func testWorkStartedFromAPlanWithTheCodeHasFullAccess() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject(
            "demo", [Repository(name: "app", path: "/repos/app")], "")
        let plan = try await daemon.client.start(
            StartRequest(backend: .scripted, projectID: project.id, planning: true, withCode: true))
        let item = try await daemon.client.createItem(project.id, ItemFields(title: "Login"))

        let work = try await daemon.client.startWork(plan.id, item.id)

        XCTAssertEqual(work.access, .full)
        XCTAssertFalse(work.planning)
    }

    func testTheProjectKeepsWhetherPlanningHasTheCode() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject(
            "demo", [Repository(name: "app", path: "/repos/app")], "")
        XCTAssertFalse(project.planWithCode)
        let updated = try await daemon.client.updateProject(project.id, ProjectUpdate(planWithCode: true))
        XCTAssertTrue(updated.planWithCode)
        let projects = try await daemon.client.projects()
        XCTAssertEqual(projects.first?.planWithCode, true)
    }
}
