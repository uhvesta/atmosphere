import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrCore
import LifrStoreClient
import XCTest

final class AssistantWriteTests: XCTestCase {
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

    private func home() -> (RecordsBuilder, LifrTask, LifrTask) {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        let rent = records.task("Pay rent", in: home)
        let plants = records.task("Water plants", in: home)
        return (records, rent, plants)
    }

    func testUpdatingTasksSavesAsTheAssistantInTheConversation() async throws {
        let (records, rent, plants) = home()
        let (tools, store) = tools(records.workspace)
        let codes = [rent, plants].map { records.workspace.displayCode(of: $0) }

        _ = await tools.updateTasks(codes: codes, status: "in_progress", priority: "high", due: "friday", title: nil)

        let saved = try await store.load()
        for task in [rent, plants] {
            XCTAssertEqual(saved.tasks[task.id]?.status, .inProgress)
            XCTAssertEqual(saved.tasks[task.id]?.priority, .high)
            XCTAssertEqual(saved.tasks[task.id]?.dueDate, try CalendarDate(year: 2026, month: 11, day: 6))
        }
        let entry = try XCTUnwrap(saved.journal.last)
        XCTAssertEqual(entry.actor, .assistant(device: "iPhone"))
        XCTAssertEqual(entry.conversation, conversation)
        XCTAssertEqual(tools.workspace, saved, "later lookups see the change")
    }

    func testAWrongCodeOrValueChangesNothing() async throws {
        let (records, rent, _) = home()
        let (tools, store) = tools(records.workspace)
        let code = records.workspace.displayCode(of: rent)

        _ = await tools.updateTasks(codes: [code, "HOME-9999"], status: "done", priority: nil, due: nil, title: nil)
        _ = await tools.updateTasks(codes: [code], status: "procrastinated", priority: nil, due: nil, title: nil)
        _ = await tools.updateTasks(codes: [code], status: nil, priority: nil, due: "someday", title: nil)

        let saved = try await store.load()
        XCTAssertEqual(saved.journal, [])
        XCTAssertEqual(saved.tasks[rent.id]?.status, .todo)
    }

    func testAddingACommentCreditsTheAssistant() async throws {
        let (records, rent, _) = home()
        let (tools, store) = tools(records.workspace)

        _ = await tools.addComment(code: records.workspace.displayCode(of: rent), text: "Paid by transfer")

        let comments = try await store.load().comments
        let comment = try XCTUnwrap(comments.values.first)
        XCTAssertEqual(comment.body, "Paid by transfer")
        XCTAssertEqual(comment.author, .assistant(device: "iPhone"))
    }

    func testRevertingTheConversationPutsBackOnlyItsChanges() async throws {
        let (records, rent, plants) = home()
        let (tools, store) = tools(records.workspace)
        let rentCode = records.workspace.displayCode(of: rent)
        _ = await tools.updateTasks(codes: [rentCode], status: "done", priority: nil, due: nil, title: nil)
        let byHand = try await withDependencies {
            $0.date.now = now
            $0.uuid = .incrementing
        } operation: {
            try await store.edit(plants.id, .init(priority: .low)).workspace
        }
        tools.workspace = byHand

        _ = await tools.revert(ref: "conversation")

        let saved = try await store.load()
        XCTAssertEqual(saved.tasks[rent.id]?.status, .todo)
        XCTAssertEqual(saved.tasks[plants.id]?.priority, .low)
    }

    func testRecentActivityGivesRefsThatRevertTakes() async throws {
        let (records, rent, _) = home()
        let (tools, store) = tools(records.workspace)
        _ = await tools.updateTasks(
            codes: [records.workspace.displayCode(of: rent)], status: nil, priority: nil, due: nil,
            title: "Pay May rent")

        let activity = tools.recentActivity()
        let ref = try XCTUnwrap(activity.split(separator: " ").drop { $0 != "ref" }.dropFirst().first)
        XCTAssertTrue(activity.contains("you"))
        _ = await tools.revert(ref: String(ref))

        let title = try await store.load().tasks[rent.id]?.title
        XCTAssertEqual(title, "Pay rent")
        let unknown = await tools.revert(ref: "zzzzzz")
        XCTAssertTrue(unknown.contains("no change"))
    }

    func testWithoutWritesTheToolsChangeNothing() async throws {
        let (records, rent, _) = home()
        let (tools, store) = tools(records.workspace, writes: false)

        _ = await tools.updateTasks(
            codes: [records.workspace.displayCode(of: rent)], status: "done", priority: nil, due: nil, title: nil)

        let journal = try await store.load().journal
        XCTAssertEqual(journal, [])
    }

    func testThePromptOffersChangesOnlyWithWrites() {
        let request = AssistantRequest(messages: [], workspace: Workspace(), now: now, timeZone: .gmt)
        var writing = request
        writing.writes = AssistantWrites(conversation: conversation) { _ in Workspace() }

        XCTAssertFalse(AssistantPrompt.instructions(request).contains("updateTasks"))
        XCTAssertTrue(AssistantPrompt.instructions(writing).contains("updateTasks"))
    }
}
