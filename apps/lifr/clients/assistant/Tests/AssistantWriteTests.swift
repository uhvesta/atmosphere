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

    /// Tools on `workspace`, and an in-memory store of it the person saves
    /// to on an iPhone.
    private func tools(_ workspace: Workspace, writes: Bool = true) -> (AssistantTools, StoreClient) {
        let store = StoreClient.inMemory(workspace, actor: .user(device: "iPhone", surface: .iPhone))
        return (tools(on: workspace, writes: writes), store)
    }

    private func tools(on workspace: Workspace, writes: Bool = true) -> AssistantTools {
        AssistantTools(
            workspace: workspace, now: now, timeZone: .gmt,
            writes: writes ? AssistantWrites(conversation: conversation) : nil)
    }

    /// Applies what `tools` proposed, as the person does, and returns the
    /// records saved.
    private func apply(_ tools: AssistantTools, to store: StoreClient) async throws -> Workspace {
        try await tools.applied(to: store, now: now, conversation: conversation)
    }

    private func home() -> (RecordsBuilder, LifrTask, LifrTask) {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        let rent = records.task("Pay rent", in: home)
        let plants = records.task("Water plants", in: home)
        return (records, rent, plants)
    }

    func testChangesWaitForThePersonThenSaveAsOneChangeByTheAssistant() async throws {
        let (records, rent, plants) = home()
        let (tools, store) = tools(records.workspace)
        let codes = [rent, plants].map { records.workspace.displayCode(of: $0) }

        for code in codes {
            _ = try await tools.change(TaskChange(code: code, status: "in_progress", priority: "high", due: "friday"))
        }

        let before = try await store.load()
        XCTAssertEqual(before.journal, [], "nothing is saved until the person applies it")
        XCTAssertEqual(tools.proposal?.steps.map(\.record), [rent, plants].map { RecordID(.task, $0.id) })
        let saved = try await apply(tools, to: store)
        XCTAssertEqual(saved.journal.count, 1)
        for task in [rent, plants] {
            XCTAssertEqual(saved.tasks[task.id]?.status, .inProgress)
            XCTAssertEqual(saved.tasks[task.id]?.priority, .high)
            XCTAssertEqual(saved.tasks[task.id]?.dueDate, try CalendarDate(year: 2026, month: 11, day: 6))
        }
        let entry = try XCTUnwrap(saved.journal.last)
        XCTAssertEqual(entry.actor, .assistant(device: "iPhone"))
        XCTAssertEqual(entry.conversation, conversation)
        XCTAssertEqual(tools.workspace.tasks, saved.tasks, "later lookups see the change")
    }

    func testAProposalForATaskTrashedSinceCantBeApplied() async throws {
        let (records, rent, _) = home()
        let (tools, store) = tools(records.workspace)
        _ = try await tools.change(TaskChange(code: records.workspace.displayCode(of: rent), status: "done"))
        let actions = try XCTUnwrap(tools.proposal?.actions)
        let trashed = try await withDependencies {
            $0.date.now = now
            $0.uuid = .incrementing
        } operation: {
            try await store.moveToTrash(rent.id).workspace
        }

        do {
            _ = try await Proposal.make(actions, on: trashed, now: now, timeZone: .gmt, conversation: conversation)
            XCTFail("a trashed task can't change")
        } catch .task(.noTask) {
        } catch {
            XCTFail("\(error)")
        }
    }

    func testAWrongCodeOrValueChangesNothing() async throws {
        let (records, rent, _) = home()
        let (tools, store) = tools(records.workspace)
        let code = records.workspace.displayCode(of: rent)

        let changes = [
            TaskChange(code: "HOME-9999", status: "done"),
            TaskChange(code: code, title: "Pay May rent", status: "procrastinated"),
            TaskChange(code: code, priority: "high", due: "someday"),
            TaskChange(code: code, place: "GARDEN"),
            TaskChange(code: code),
        ]
        var errors: [TaskChangeError] = []
        for change in changes {
            do {
                _ = try await tools.change(change)
            } catch {
                errors.append(error)
            }
        }

        XCTAssertEqual(
            errors,
            [
                .noTask(code: "HOME-9999"), .unknownStatus("procrastinated"), .invalidDate("someday"),
                .unknownAvenue(key: "GARDEN", suggestion: nil), .nothingToChange(code: code),
            ])
        let saved = try await apply(tools, to: store)
        XCTAssertEqual(saved.journal, [])
        XCTAssertEqual(saved.tasks[rent.id]?.status, .todo)
    }

    func testAddingACommentCreditsTheAssistant() async throws {
        let (records, rent, _) = home()
        let (tools, store) = tools(records.workspace)

        _ = await tools.addComment(code: records.workspace.displayCode(of: rent), text: "Paid by transfer")

        let comments = try await apply(tools, to: store).comments
        let comment = try XCTUnwrap(comments.values.first)
        XCTAssertEqual(comment.body, "Paid by transfer")
        XCTAssertEqual(comment.author, .assistant(device: "iPhone"))
    }

    func testRevertingTheConversationPutsBackOnlyItsChanges() async throws {
        let (records, rent, plants) = home()
        let (tools, store) = tools(records.workspace)
        let rentCode = records.workspace.displayCode(of: rent)
        _ = try await tools.change(TaskChange(code: rentCode, status: "done"))
        _ = try await apply(tools, to: store)
        let byHand = try await withDependencies {
            $0.date.now = now
            $0.uuid = .incrementing
        } operation: {
            try await store.edit(plants.id, .init(priority: .low)).workspace
        }
        let later = self.tools(on: byHand)

        _ = await later.revert(ref: "conversation")

        let saved = try await apply(later, to: store)
        XCTAssertEqual(saved.tasks[rent.id]?.status, .todo)
        XCTAssertEqual(saved.tasks[plants.id]?.priority, .low)
    }

    func testRecentActivityGivesRefsThatRevertTakes() async throws {
        let (records, rent, _) = home()
        let (tools, store) = tools(records.workspace)
        _ = try await tools.change(TaskChange(code: records.workspace.displayCode(of: rent), title: "Pay May rent"))
        let later = self.tools(on: try await apply(tools, to: store))

        let activity = later.recentActivity()
        let ref = try XCTUnwrap(activity.split(separator: " ").drop { $0 != "ref" }.dropFirst().first)
        XCTAssertTrue(activity.contains("you"))
        _ = await later.revert(ref: String(ref))

        let title = try await apply(later, to: store).tasks[rent.id]?.title
        XCTAssertEqual(title, "Pay rent")
        let unknown = await later.revert(ref: "zzzzzz")
        XCTAssertTrue(unknown.contains("no change"))
    }

    func testWithoutWritesTheToolsChangeNothing() async throws {
        let (records, rent, _) = home()
        let (tools, store) = tools(records.workspace, writes: false)

        await assertThrowsAsync(
            try await tools.change(TaskChange(code: records.workspace.displayCode(of: rent), status: "done")),
            .readOnly)

        let journal = try await apply(tools, to: store).journal
        XCTAssertEqual(journal, [])
    }

    func testThePromptOffersChangesOnlyWithWrites() {
        let request = AssistantRequest(messages: [], workspace: Workspace(), now: now, timeZone: .gmt, skill: .change)
        var writing = request
        writing.writes = AssistantWrites(conversation: conversation)

        XCTAssertFalse(request.changesTasks)
        XCTAssertTrue(writing.changesTasks)
        XCTAssertFalse(AssistantPrompt.tools(writing, size: .full).contains(.updateTasks), "changes are in the answer")
    }

    func testMovingATaskTakesItsSubTasksToTheAvenueOrProject() async throws {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        let work = records.avenue("WORK", "Work")
        let launch = records.project("Launch", in: work)
        let passport = records.task("Renew passport", in: home)
        let photo = records.task("Get a photo", in: home, parent: passport)
        let (tools, store) = tools(records.workspace)
        let code = records.workspace.displayCode(of: passport)

        _ = try await tools.change(TaskChange(code: code, place: "work"))
        var saved = try await apply(tools, to: store)
        XCTAssertEqual(saved.tasks[passport.id]?.avenueID, work.id)
        XCTAssertEqual(saved.tasks[photo.id]?.avenueID, work.id)

        _ = try await tools.change(TaskChange(code: code, place: "Work/launch"))
        saved = try await apply(tools, to: store)
        XCTAssertEqual(saved.tasks[passport.id]?.projectID, launch.id)

        let photoCode = saved.displayCode(of: try XCTUnwrap(saved.tasks[photo.id]))
        await assertThrowsAsync(
            try await tools.change(TaskChange(code: photoCode, place: "HOME")), .subtaskMoves(code: photoCode))
    }

    func testAChangeToWhatTheTaskHasAlreadyChangesNothing() async throws {
        let (records, rent, _) = home()
        let (tools, store) = tools(records.workspace)
        let code = records.workspace.displayCode(of: rent)

        let same = try await tools.change(TaskChange(code: code, title: "Pay rent", status: "todo", priority: "none"))
        let renamed = try await tools.change(TaskChange(code: code, title: "Pay the rent", status: "todo"))

        XCTAssertEqual(same, rent)
        XCTAssertEqual(renamed.title, "Pay the rent")
        XCTAssertEqual(renamed.status, .todo)
        let journal = try await apply(tools, to: store).journal
        XCTAssertEqual(journal.count, 1)
    }

    func testFinishingAndReschedulingInOneAnswer() async throws {
        let (records, rent, plants) = home()
        let (tools, store) = tools(records.workspace)

        let done = try await tools.change(TaskChange(code: records.workspace.displayCode(of: plants), status: "Done"))
        let moved = try await tools.change(
            TaskChange(code: records.workspace.displayCode(of: rent).lowercased(), due: "next monday"))

        XCTAssertEqual(done.status, .done)
        XCTAssertEqual(moved.dueDate?.description, "2026-11-09")
        let saved = try await apply(tools, to: store)
        XCTAssertEqual(saved.tasks[rent.id]?.dueDate?.description, "2026-11-09")
    }

    func testRenamingOrReprioritisingKeepsTheDueDateAndTime() async throws {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        let call = records.task("Call the bank", in: home, due: 2, at: "15:00")
        let (tools, store) = tools(records.workspace)
        let code = records.workspace.displayCode(of: call)

        _ = try await tools.change(TaskChange(code: code, title: "Call the bank about the card", priority: "high"))

        let saved = try await apply(tools, to: store).tasks[call.id]
        XCTAssertEqual(saved?.title, "Call the bank about the card")
        XCTAssertEqual(saved?.priority, .high)
        XCTAssertEqual(saved?.dueDate, call.dueDate)
        XCTAssertEqual(saved?.dueTime, call.dueTime)
    }

    func testNoDueDateClearsATimedTasksDateAndTime() async throws {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        let call = records.task("Call the bank", in: home, due: 2, at: "15:00")
        let (tools, store) = tools(records.workspace)

        _ = try await tools.change(TaskChange(code: records.workspace.displayCode(of: call), due: "none"))

        let saved = try await apply(tools, to: store).tasks[call.id]
        XCTAssertNil(saved?.dueDate)
        XCTAssertNil(saved?.dueTime)
    }
}

/// Fails unless `body` throws `expected`.
private func assertThrowsAsync<T>(
    _ body: @autoclosure () async throws(TaskChangeError) -> T, _ expected: TaskChangeError,
    file: StaticString = #filePath, line: UInt = #line
) async {
    do {
        _ = try await body()
        XCTFail("Didn't throw \(expected)", file: file, line: line)
    } catch {
        XCTAssertEqual(error, expected, file: file, line: line)
    }
}

final class MeantTasksTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func request(_ text: String) -> AssistantRequest {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        records.task("Water the plants", in: home)
        records.task("Call the plumber", in: home)
        records.task("Book physio appointment", in: home)
        return AssistantRequest(
            messages: [ChatMessage(id: UUID(), role: .person, text: text, at: now)], workspace: records.workspace,
            now: now, timeZone: .gmt, skill: .change)
    }

    func testTheTasksAMessageNamesAreListedWithTheirCodes() {
        let meant = AssistantPrompt.meantTasks(request("I watered the plants, mark it done"))
        XCTAssertEqual(meant.map(\.title), ["Water the plants"])
        let code = meant.first?.code ?? "none"
        XCTAssertTrue(AssistantPrompt.prompt(request("I watered the plants, mark it done")).contains(code))
    }

    func testAMessageSharingOneWordOfALongerTitleListsNothing() {
        XCTAssertEqual(AssistantPrompt.meantTasks(request("Add a task to call my mum")).map(\.title), [])
        XCTAssertEqual(AssistantPrompt.meantTasks(request("Change that to Friday")).map(\.title), [])
    }
}
