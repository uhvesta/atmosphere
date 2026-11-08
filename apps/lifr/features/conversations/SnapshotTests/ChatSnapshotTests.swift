import ChatUI
import ComposableArchitecture
import FlowKit
import InferenceKit
import LifrAssistantClient
import LifrConversations
import LifrCore
import LifrModelsClient
import LifrStoreClient
import SnapshotTesting
import SwiftUI
import XCTest

/// A chat about a garden, with the assistant's draft open on the card, and
/// one using skills, for the sample records as of 2026-11-03 10:00
/// UTC.
@MainActor
final class ChatSnapshotTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func chat(problem: String? = nil) -> ChatFeature.State {
        var conversation = Conversation(id: UUID(0), createdAt: now)
        conversation.messages = [
            ChatMessage(id: UUID(1), role: .person, text: "Plan a vegetable garden this month", at: now),
            ChatMessage(
                id: UUID(2), role: .assistant, text: "Here's a project for it. Edit anything before adding.",
                at: now),
        ]
        conversation.draft = Plan(
            projects: [
                Plan.ProjectDraft(
                    name: "Vegetable garden", avenue: "HOME", due: "2026-11-30",
                    tasks: [
                        Plan.TaskDraft(title: "Buy soil", priority: .high, due: "2026-11-07"),
                        Plan.TaskDraft(
                            title: "Plant seedlings",
                            subtasks: [Plan.TaskDraft(title: "Tomatoes")]),
                    ])
            ],
            tasks: [Plan.TaskDraft(title: "Ask about a water butt", place: "HOME")])
        var state = ChatFeature.State(conversation: conversation, workspace: .sample(now: now, timeZone: .gmt))
        state.problem = problem
        return state
    }

    /// A short chat whose first message used Review, with Tidy picked for
    /// the next one.
    private var skills: ChatFeature.State {
        var conversation = Conversation(id: UUID(0), createdAt: now)
        conversation.messages = [
            ChatMessage(id: UUID(1), role: .person, text: "What's overdue at home?", at: now, skill: "review"),
            ChatMessage(
                id: UUID(2), role: .assistant, text: "Two tasks: Pay rent and Fix the tap. Want new dates for them?",
                at: now),
        ]
        var state = ChatFeature.State(conversation: conversation, workspace: .sample(now: now, timeZone: .gmt))
        state.skill = "tidy"
        return state
    }

    /// A chat with a model that reads photos and recordings: a sent photo
    /// that's on another device, and a photo and a recording waiting to go
    /// with the next message. `recording` while the microphone records.
    private func attachments(recording: Bool = false) -> ChatFeature.State {
        var conversation = Conversation(id: UUID(0), createdAt: now)
        conversation.messages = [
            ChatMessage(
                id: UUID(1), role: .person, text: "What plant is this?", at: now,
                attachments: [ChatAttachment(id: UUID(10), kind: .image)]),
            ChatMessage(
                id: UUID(2), role: .assistant, text: "A Boston fern. It likes indirect light and damp soil.", at: now),
        ]
        var state = ChatFeature.State(conversation: conversation, workspace: .sample(now: now, timeZone: .gmt))
        state.inputs = [.image, .audio]
        state.elsewhere = [UUID(10)]
        state.pending = [
            ChatAttachment(id: UUID(11), kind: .image), ChatAttachment(id: UUID(12), kind: .audio, seconds: 7),
        ]
        state.recording = recording
        state.input = "Add a task to water it"
        return state
    }

    /// A chat whose older messages were summarized, and whose last reply
    /// was written without some of those after: Compact is offered.
    private var compacted: ChatFeature.State {
        var conversation = Conversation(id: UUID(0), createdAt: now)
        conversation.messages = [
            ChatMessage(id: UUID(1), role: .person, text: "Plan a vegetable garden this month", at: now),
            ChatMessage(id: UUID(2), role: .assistant, text: "Added a Vegetable garden Project in HOME.", at: now),
            ChatMessage(id: UUID(3), role: .person, text: "Move Buy soil to Saturday", at: now),
            ChatMessage(id: UUID(4), role: .assistant, text: "Buy soil is due Saturday now.", at: now),
        ]
        conversation.summaries[UUID(2)] = ChatSummary(
            text: "The person planned a vegetable garden for November: a Vegetable garden Project in HOME, due "
                + "November 30, with Buy soil and Plant seedlings.",
            at: now)
        var state = ChatFeature.State(conversation: conversation, workspace: .sample(now: now, timeZone: .gmt))
        state.leftOut = 1
        return state
    }

    /// A chat with nothing in it yet and / typed, listing the skills.
    private var slash: ChatFeature.State {
        var state = ChatFeature.State(
            conversation: Conversation(id: UUID(0), createdAt: now), workspace: .sample(now: now, timeZone: .gmt))
        state.input = "/"
        return state
    }

    /// Apple's model and a downloaded Qwen3.5, which the sheet offers.
    private func models() throws -> [RunnableModel] {
        let entry = ModelCatalog.qwen35Small
        let variant = try XCTUnwrap(entry.variants.first)
        let qwen = InstalledModel(spec: entry.running(variant), directory: URL(filePath: "/"), installedAt: now)
        return [.apple, RunnableModel(qwen)]
    }

    /// The chat on Apple's model, the default, with a third of its window
    /// used by the last reply.
    private func screen(_ state: ChatFeature.State) -> some View {
        var state = state
        state.models = [.apple]
        state.standard = .apple
        if !state.conversation.messages.isEmpty { state.context = ContextUse(used: 1_400, window: 4_096) }
        return NavigationStack {
            ChatView(store: Store(initialState: state) { EmptyReducer() })
        }
        .environment(\.timeZone, .gmt)
    }

    /// Only the screens the whole chat fits on: a chat that scrolls renders
    /// blank off screen.
    func testImages() {
        assertScreenImages(of: screen(chat()), named: "draft", images: [.proMaxLight, .proMaxDark])
        assertScreenImages(
            of: screen(chat(problem: "There's no Avenue with the key HOM.")), named: "problem", images: [.proMaxLight])
        assertScreenImages(of: screen(skills), named: "skills")
        assertScreenImages(of: screen(attachments()), named: "attachments")
        assertScreenImages(of: screen(attachments(recording: true)), named: "recording", images: [.proMaxLight])
        assertScreenImages(of: screen(compacted), named: "compacted", images: [.proMaxLight, .proMaxDark])
        assertScreenImages(of: screen(slash), named: "slash", images: [.proMaxLight, .proMaxDark])
    }

    /// The sheet picking the chat's model, on Qwen3.5 thinking long.
    func testModelSheet() throws {
        let sheet = ModelSheet(
            model: .constant(ModelCatalog.qwen35Small.id), thinking: .constant(.long), window: .constant(nil),
            models: try models(), standard: .apple, done: {})
        assertScreenImages(of: sheet, named: "model", images: [.proMaxLight, .proMaxDark, .seLargestText])
    }
}
