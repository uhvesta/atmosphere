import ComposableArchitecture
import FlowKit
import LifrAssistantClient
import LifrAssistantLive
import LifrCore
import LifrGlanceClient
import LifrLiteRT
import LifrLockClient
import LifrModelsClient
import LifrModelsLive
import LifrRemindersClient
import LifrRoot
import LifrSessionClient
import LifrStoreClient
import LifrSyncClient
import SwiftUI
import XCTest

/// The assistant on a real model: Gemma 4 E2B on this Mac's GPU, through
/// LiteRT-LM and the live app, on the sample records as of 2026-11-03
/// 10:00 UTC.
///
/// The model's words differ from one GPU to the next, even taking the
/// likeliest token each time, so the trees show each run of them as
/// `‹model text›` and the steps that show them have no image. What the
/// test checks is that the screen moved on from "Thinking…" to a reply and
/// a draft, that adding the draft added tasks, and that Undo took them
/// away again.
@MainActor
final class AssistantOnRealModelTests: XCTestCase {
    override func setUp() async throws {
        try FlowRecorder.checkScreen()
    }

    /// Asking for a task: the reply and its draft, fixing the draft, adding
    /// it, then Undo.
    func testAReplyItsDraftAndUndo() async throws {
        let model = try RealModel()
        let store = model.store
        let flow = FlowRecorder(
            "real-model", RootView(store: store).environment(\.timeZone, .gmt), redactsModelText: true)
        defer { flow.finish() }
        let id = try await model.chat()
        await flow.step("new")

        try await model.send("Add a task to buy milk tomorrow, in HOME")
        let draft = try XCTUnwrap(model.draft, "The model answered without a draft")

        // A small model's draft often can't be added as it is: a task with
        // no place, or a date it got wrong. The person fixes it in the
        // editor, keeping the model's titles, so what follows doesn't
        // depend on how well this GPU's model did.
        let fixed = Self.fixed(draft)
        XCTAssertGreaterThan(fixed.taskCount, 0, "The model's draft adds no task: \(draft)")
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.conversation.draft, fixed))))))
        let chat = try XCTUnwrap(model.chatState)
        XCTAssertNil(chat.problem, "The fixed draft can't be added: \(chat.problem ?? "")")
        await flow.step("fixed-draft", image: false)

        let added = try await model.addDraft()
        XCTAssertEqual(added.count, fixed.taskCount, "Adding the draft added a different number of tasks")

        let before = try model.liveTasks().subtracting(added.map(\.id))
        let last = try XCTUnwrap(store.lastChange)
        store.send(.undoTapped(last.id))
        try await model.until("Undo takes them away", timeout: .seconds(10)) { store.lastChange == nil }
        XCTAssertEqual(try model.liveTasks(), before, "Undo left tasks the draft added")
        await flow.step("undone", image: false)
    }

    /// `draft`'s tasks, those of its Projects too, each in Home with
    /// nothing else set.
    private static func fixed(_ draft: Plan) -> Plan {
        var fixed = draft
        fixed.avenues = []
        fixed.labels = []
        fixed.projects = []
        fixed.tasks = (draft.tasks + draft.projects.flatMap(\.tasks)).map { task in
            var task = task
            task.place = "HOME"
            task.parent = nil
            task.start = nil
            task.due = nil
            task.dueTime = nil
            task.labels = []
            task.reminders = []
            task.subtasks = []
            return task
        }
        return fixed
    }
}
