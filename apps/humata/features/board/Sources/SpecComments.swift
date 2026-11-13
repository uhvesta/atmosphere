import ChatUI
import ComposableArchitecture
import Foundation
import HumataDaemonClient
import HumataReview
import SwiftUI

/// Comments on parts of a task's spec, as in a chat's files: pick a block,
/// write what's wrong with it, and send every comment to one of the task's
/// sessions as one message.
@Reducer
package struct SpecCommentsFeature {
    package struct Draft: Hashable, Sendable, Identifiable {
        package var id: UUID
        package var block: MarkdownBlock
        package var text: String

        package init(id: UUID, block: MarkdownBlock, text: String) {
            self.id = id
            self.block = block
            self.text = text
        }
    }

    @ObservableState
    package struct State: Equatable {
        package var title: String
        package var picked: MarkdownBlock?
        package var comment = ""
        package var drafts: IdentifiedArrayOf<Draft> = []
        package var sending = false
        @Presents package var alert: AlertState<Never>?

        package init(title: String) {
            self.title = title
        }

        /// The drafts as the one message they're sent as, in the spec's
        /// order: each block quoted, then the comment on it.
        package var message: String {
            let comments = drafts.sorted { $0.block.startLine < $1.block.startLine }.map { draft in
                let quote = draft.block.text.split(separator: "\n", omittingEmptySubsequences: false)
                    .map { "> \($0)" }.joined(separator: "\n")
                return quote + "\n" + draft.text
            }
            let intro =
                drafts.count == 1
                ? "A comment on the spec of \(title):"
                : "Comments on the spec of \(title); answer each:"
            return ([intro] + comments).joined(separator: "\n\n")
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case blockTapped(MarkdownBlock)
        case commentCancelled
        case commentSaved
        case draftRemoved(Draft.ID)
        case sendTapped(to: Session.ID)
        case sent
        case failed(String)
        case alert(PresentationAction<Never>)
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.uuid) var uuid

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding, .alert:
                return .none

            case .blockTapped(let block):
                state.picked = state.picked == block ? nil : block
                state.comment = ""
                return .none

            case .commentCancelled:
                state.picked = nil
                state.comment = ""
                return .none

            case .commentSaved:
                let text = state.comment.trimmingCharacters(in: .whitespacesAndNewlines)
                guard let block = state.picked, !text.isEmpty else { return .none }
                state.drafts.append(Draft(id: uuid(), block: block, text: text))
                state.picked = nil
                state.comment = ""
                return .none

            case .draftRemoved(let id):
                state.drafts.remove(id: id)
                return .none

            case .sendTapped(let session):
                guard !state.drafts.isEmpty, !state.sending else { return .none }
                state.sending = true
                let message = state.message
                return .run { send in
                    try await daemon.send(session, message, .enqueue)
                    await send(.sent)
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .sent:
                state.sending = false
                state.drafts = []
                return .none

            case .failed(let message):
                // The comments stay, to send again or to another session.
                state.sending = false
                state.alert = AlertState {
                    TextState("Couldn't send the comments")
                } message: {
                    TextState(message)
                }
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }
}

/// A spec's blocks to comment on, its comments so far, and where to send them.
struct SpecCommentsView: View {
    @Bindable var store: StoreOf<SpecCommentsFeature>
    let spec: String
    let sessions: [Session]
    @State private var target: Session.ID?

    var body: some View {
        let blocks = MarkdownBlock.cached(spec)
        ForEach(blocks) { block in
            VStack(alignment: .leading, spacing: 6) {
                Button {
                    store.send(.blockTapped(block))
                } label: {
                    Group {
                        // A paragraph may be a list, which only the full
                        // renderer draws as one.
                        if block.kind == .paragraph {
                            MarkdownText(block.text)
                        } else {
                            MarkdownBlockView(block: block)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Comment on this part of the spec")
                if store.drafts.contains(where: { $0.block == block }) {
                    Label("Commented", systemImage: "text.bubble")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                if store.picked == block {
                    TextField(
                        "Comment", text: $store.comment, prompt: Text("What should change here?"), axis: .vertical
                    )
                    .lineLimit(2...6)
                    .labelsHidden()
                    .onSubmit { store.send(.commentSaved) }
                    HStack {
                        Spacer()
                        Button("Cancel") { store.send(.commentCancelled) }
                        Button("Add Comment") { store.send(.commentSaved) }
                            .disabled(store.comment.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
        }
        if !store.drafts.isEmpty {
            ForEach(store.drafts) { draft in
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(draft.block.text).lineLimit(1).foregroundStyle(.secondary)
                        Text(draft.text)
                    }
                    Spacer()
                    Button {
                        store.send(.draftRemoved(draft.id))
                    } label: {
                        Label("Remove", systemImage: "minus.circle").labelStyle(.iconOnly)
                    }
                    .buttonStyle(.borderless)
                    .help("Remove this comment")
                }
            }
            if sessions.isEmpty {
                Text("Start a session on this task to send the comments to its agent.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                HStack {
                    Picker("Send To", selection: $target) {
                        ForEach(sessions) { session in
                            Text(session.name.isEmpty ? "Session" : session.name).tag(Optional(session.id))
                        }
                    }
                    Button(store.drafts.count == 1 ? "Send Comment" : "Send \(store.drafts.count) Comments") {
                        if let target = target ?? sessions.first?.id {
                            store.send(.sendTapped(to: target))
                        }
                    }
                    .disabled(store.sending)
                }
                .onAppear { target = target ?? sessions.first?.id }
            }
        }
    }
}
