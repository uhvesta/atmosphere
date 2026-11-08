import ComposableArchitecture
import FlowKit
import LifrAPI
import LifrAssistantClient
import LifrAssistantLive
import LifrBugReport
import LifrConversations
import LifrCore
import LifrGlance
import LifrGlanceClient
import LifrLockClient
import LifrModelsClient
import LifrRemindersClient
import LifrRoot
import LifrSearchClient
import LifrSessionClient
import LifrShortcuts
import LifrStoreClient
import LifrSyncClient
import LifrWidgetViews
import SwiftUI
import XCTest

/// The assistant on the iPhone changing tasks and projects that are there, and a change declined.
@MainActor
final class PhoneAssistantEditsFlowTests: PhoneFlowTestCase {
    /// Pushing a task to next Monday, where the model works the date out
    /// as the Saturday before: the task is due the Monday the person named.
    func testAssistantReschedulesToTheDayNamed() async throws {
        let workspace = Workspace.sample(now: now, timeZone: .gmt)
        let plumber = try XCTUnwrap(workspace.tasks.values.first { $0.title == "Call the plumber" })
        let code = JSONValue.string(workspace.displayCode(of: plumber))
        let (store, flow) = app(
            "assistant-reschedule", workspace: workspace,
            assistant: gemma(
                [
                    [
                        "changes": [["code": code, "due": "2026-11-07"]],
                        "message": "I moved Call the plumber to next Monday.",
                    ]
                ], picking: Skill.change.id))
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        await flow.step("new-chat", image: false)
        send("Push “Call the plumber” to next Monday", in: id, of: store)
        await flow.step("proposed", image: false)
        let reply = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last?.id)
        store.send(.conversations(.chats(.element(id: id, action: .applyTapped(messageID: reply)))))
        await flow.step("rescheduled")
        let task = store.workspace?.tasks.values.first { $0.title == "Call the plumber" }
        XCTAssertEqual(task?.dueDate?.description, "2026-11-09")
        flow.finish()
    }

    /// Renaming a task, where the model writes the old title back as the
    /// new one and a status it already has: the task gets the quoted title.
    func testAssistantRenamesATask() async throws {
        let workspace = Workspace.sample(now: now, timeZone: .gmt)
        let plumber = try XCTUnwrap(workspace.tasks.values.first { $0.title == "Call the plumber" })
        let code = JSONValue.string(workspace.displayCode(of: plumber))
        let (store, flow) = app(
            "assistant-rename", workspace: workspace,
            assistant: gemma(
                [
                    [
                        "changes": [["code": code, "status": "todo", "rename": "Call the plumber"]],
                        "message": "I renamed the task to Call the electrician.",
                    ]
                ], picking: Skill.change.id))
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        // Once the chat has the records, so it knows the task.
        await flow.step("new-chat", image: false)
        send("Rename “Call the plumber” to “Call the electrician”", in: id, of: store)
        await flow.step("proposed", image: false)
        let reply = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last?.id)
        store.send(.conversations(.chats(.element(id: id, action: .applyTapped(messageID: reply)))))
        await flow.step("renamed")
        let titles = store.workspace?.tasks.values.map(\.title) ?? []
        XCTAssertTrue(titles.contains("Call the electrician"))
        XCTAssertFalse(titles.contains("Call the plumber"))
        flow.finish()
    }

    /// Asking the assistant to rename the Half marathon and set when
    /// it's due: the change it proposes, Apply, then Undo on its reply.
    func testAssistantEditsAProject() async throws {
        let workspace = Workspace.sample(now: now, timeZone: .gmt)
        let marathon = try XCTUnwrap(workspace.projects.values.first { $0.name == "Half marathon" })
        let (store, flow) = app(
            "assistant-edits-project", workspace: workspace,
            assistant: gemma(
                [
                    [
                        "projectChanges": [
                            ["project": "HEALTH/Half marathon", "rename": "Spring half marathon", "due": "2026-12-06"]
                        ],
                        "message": "I renamed it to Spring half marathon, due 6 December.",
                    ]
                ], picking: Skill.change.id))
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        await flow.step("new-chat", image: false)
        send("Rename the Half marathon to “Spring half marathon”, due 2026-12-06", in: id, of: store)
        await flow.step("proposed")
        XCTAssertEqual(store.workspace?.projects[marathon.id]?.name, "Half marathon", "nothing changes yet")
        let reply = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last?.id)

        store.send(.conversations(.chats(.element(id: id, action: .applyTapped(messageID: reply)))))
        await flow.step("edited")
        XCTAssertEqual(store.workspace?.projects[marathon.id]?.name, "Spring half marathon")
        XCTAssertEqual(store.workspace?.projects[marathon.id]?.dueDate?.description, "2026-12-06")

        store.send(.conversations(.chats(.element(id: id, action: .undoTapped(messageID: reply)))))
        await flow.step("undone")
        XCTAssertEqual(store.workspace?.projects[marathon.id]?.name, "Half marathon")
        XCTAssertEqual(store.workspace?.projects[marathon.id]?.dueDate, marathon.dueDate)
        flow.finish()
    }

    /// The assistant proposing to finish a task: Decline, which changes
    /// nothing, then asked again, Edit to In Progress instead, and Apply.
    func testAssistantProposalDeclinedAndEdited() async throws {
        let workspace = Workspace.sample(now: now, timeZone: .gmt)
        let plumber = try XCTUnwrap(workspace.tasks.values.first { $0.title == "Call the plumber" })
        let code = JSONValue.string(workspace.displayCode(of: plumber))
        let done: JSONValue = ["changes": [["code": code, "status": "done"]], "message": "I'd mark it done."]
        let (store, flow) = app(
            "assistant-proposal", workspace: workspace, assistant: gemma([done, done], picking: Skill.change.id))
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        func chat(_ action: ChatFeature.Action) { store.send(.conversations(.chats(.element(id: id, action: action)))) }
        await flow.step("new-chat", image: false)
        send("I called the plumber", in: id, of: store)
        await flow.step("proposed")
        let first = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last?.id)
        chat(.declineTapped(messageID: first))
        await flow.step("declined")
        XCTAssertEqual(store.workspace?.tasks[plumber.id]?.status, plumber.status)

        send("Mark “Call the plumber” done", in: id, of: store)
        await flow.step("asked-again", image: false)
        let second = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last?.id)
        chat(.editProposalTapped(messageID: second))
        var edit = try XCTUnwrap(store.conversations.chats[id: id]?.editingProposal)
        edit.actions = [.task(TaskChange(code: workspace.displayCode(of: plumber), status: "in_progress"))]
        chat(.binding(.set(\.editingProposal, edit)))
        await flow.step("editing")
        chat(.editProposalDone)
        await flow.step("edited")
        chat(.applyTapped(messageID: second))
        await flow.step("applied")
        XCTAssertEqual(store.workspace?.tasks[plumber.id]?.status, .inProgress)
        flow.finish()
    }
}
