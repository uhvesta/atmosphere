import ComposableArchitecture
import Foundation
import LifrAPI
import LifrAssistantClient
import LifrCore
import LifrStoreClient
import XCTest

final class ProjectWriteTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private let conversation = UUID()

    /// Tools that save through an in-memory store as the person on an iPhone.
    private func tools(_ workspace: Workspace, writes: Bool = true) -> (AssistantTools, StoreClient) {
        let store = StoreClient.inMemory(workspace, actor: .user(device: "iPhone", surface: .iPhone))
        let save = AssistantWrites(conversation: conversation) { [now] operation in
            try await withDependencies {
                $0.date.now = now
                $0.uuid = .incrementing
            } operation: {
                try await store.save(operation).workspace
            }
        }
        return (AssistantTools(workspace: workspace, now: now, timeZone: .gmt, writes: writes ? save : nil), store)
    }

    /// HEALTH with the Project "Half marathon", which has one open task.
    private func health() -> (RecordsBuilder, Avenue, Project, LifrTask) {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let health = records.avenue("HEALTH", "Health")
        let marathon = records.project("Half marathon", in: health)
        let shoes = records.task("Buy running shoes", in: health, project: marathon)
        return (records, health, marathon, shoes)
    }

    func testRenamingDescribingAndReschedulingSavesAsTheAssistant() async throws {
        let (records, _, marathon, _) = health()
        let (tools, store) = tools(records.workspace)

        let changed = try await tools.change(
            ProjectChange(
                project: "HEALTH/Half marathon", name: "Spring half marathon", details: "Under two hours",
                due: "2026-11-20"))

        XCTAssertEqual(changed.name, "Spring half marathon")
        let saved = try await store.load()
        let project = try XCTUnwrap(saved.projects[marathon.id])
        XCTAssertEqual(project.name, "Spring half marathon")
        XCTAssertEqual(project.details, "Under two hours")
        XCTAssertEqual(project.dueDate?.description, "2026-11-20")
        let entry = try XCTUnwrap(saved.journal.last)
        XCTAssertEqual(entry.actor, .assistant(device: "iPhone"))
        XCTAssertEqual(entry.conversation, conversation)
    }

    func testArchivingKeepsItsOpenTasksInTheAvenue() async throws {
        let (records, health, marathon, shoes) = health()
        let (tools, store) = tools(records.workspace)

        _ = try await tools.change(ProjectChange(project: "Half marathon", status: "archived"))

        let saved = try await store.load()
        XCTAssertEqual(saved.projects[marathon.id]?.status, .archived)
        let task = try XCTUnwrap(saved.tasks[shoes.id])
        XCTAssertNil(task.projectID)
        XCTAssertEqual(task.avenueID, health.id)
        XCTAssertEqual(task.status, .todo, "the open task isn't closed")
    }

    func testCompletingWithOpenTasksIsRefusedAndChangesNothing() async throws {
        let (records, _, marathon, _) = health()
        let (tools, store) = tools(records.workspace)

        do {
            _ = try await tools.change(ProjectChange(project: "Half marathon", status: "completed"))
            XCTFail("Completed a Project with an open task")
        } catch {
            guard case .refused(_, .projectHasOpenTasks) = error else { return XCTFail("\(error)") }
        }

        let saved = try await store.load()
        XCTAssertEqual(saved.projects[marathon.id]?.status, .active)
        XCTAssertEqual(saved.journal, [])
    }

    func testReopeningAnArchivedProject() async throws {
        let (records, _, marathon, _) = health()
        let (tools, store) = tools(records.workspace)
        _ = try await tools.change(ProjectChange(project: "Half marathon", status: "archived"))

        _ = try await tools.change(ProjectChange(project: "Half marathon", status: "active"))

        let saved = try await store.load()
        XCTAssertEqual(saved.projects[marathon.id]?.status, .active)
    }

    func testAnUnknownProjectSuggestsTheClosestName() async {
        let (records, _, _, _) = health()
        let (tools, _) = tools(records.workspace)

        do {
            _ = try await tools.change(ProjectChange(project: "Half marathn", name: "Ten k"))
            XCTFail("Changed a Project that doesn't exist")
        } catch {
            XCTAssertEqual(error, .noProject(name: "Half marathn", suggestion: "Half marathon"))
        }
    }

    func testAWrongStatusOrDateChangesNothing() async throws {
        let (records, _, _, _) = health()
        let (tools, store) = tools(records.workspace)

        for change in [
            ProjectChange(project: "Half marathon", name: "Ten k", status: "paused"),
            ProjectChange(project: "Half marathon", name: "Ten k", due: "someday soonish"),
        ] {
            do {
                _ = try await tools.change(change)
                XCTFail("Made \(change)")
            } catch {
                XCTAssertTrue([.unknownStatus("paused"), .invalidDate("someday soonish")].contains(error))
            }
        }

        let journal = try await store.load().journal
        XCTAssertEqual(journal, [])
    }

    func testWithoutWritesNothingChanges() async throws {
        let (records, _, _, _) = health()
        let (tools, store) = tools(records.workspace, writes: false)

        do {
            _ = try await tools.change(ProjectChange(project: "Half marathon", name: "Ten k"))
            XCTFail("Changed a Project without writes")
        } catch {
            XCTAssertEqual(error, .readOnly)
        }

        let journal = try await store.load().journal
        XCTAssertEqual(journal, [])
    }

    private func request(_ text: String, writes: Bool = true) -> AssistantRequest {
        let (records, _, _, _) = health()
        var request = AssistantRequest(
            messages: [ChatMessage(id: UUID(), role: .person, text: text, at: now)], workspace: records.workspace,
            now: now, timeZone: .gmt)
        if writes { request.writes = AssistantWrites(conversation: conversation) { _ in Workspace() } }
        return request
    }

    func testTheProjectsAMessageNamesAreListed() {
        XCTAssertEqual(
            AssistantPrompt.meantProjects(request("Push the half marathon to December")).map(\.place),
            ["HEALTH/Half marathon"])
        XCTAssertEqual(AssistantPrompt.meantProjects(request("Add a task to run 5k")).map(\.place), [])
    }

    func testAskingToArchiveAProjectAsksForAProjectChange() {
        XCTAssertTrue(AssistantPrompt.asksForProjectChange(request("Archive the half marathon")))
        XCTAssertFalse(AssistantPrompt.asksForProjectChange(request("How is the half marathon going?")))
        XCTAssertFalse(
            AssistantPrompt.asksForProjectChange(request("Archive the half marathon", writes: false)),
            "without writes nothing can change")
    }

    func testTheAnswersProjectChangesAreReadOncePerProject() {
        let answer: JSONValue = [
            "message": "Renamed it.",
            "projectChanges": [
                ["project": "HEALTH/Half marathon", "rename": "Spring half", "due": "2026-12-01"],
                ["project": "health/half marathon", "status": "archived"],
            ],
        ]

        XCTAssertEqual(
            AssistantPrompt.projectChanges(from: answer),
            [ProjectChange(project: "HEALTH/Half marathon", name: "Spring half", due: "2026-12-01")])
    }
}
