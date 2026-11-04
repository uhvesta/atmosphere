import ComposableArchitecture
import LifrAssistantClient
import LifrConversations
import LifrCore
import LifrSnapshotTesting
import LifrStoreClient
import SnapshotTesting
import SwiftUI
import XCTest

/// A chat about a garden, with the assistant's draft open on the card, for
/// the sample records as of 2026-11-03 10:00 UTC.
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

    private func screen(_ state: ChatFeature.State) -> some View {
        NavigationStack {
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
    }
}
