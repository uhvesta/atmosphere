import ComposableArchitecture
import LifrAssistantClient
import LifrCore
import SwiftUI

package struct ChatView: View {
    @Bindable package var store: StoreOf<ChatFeature>

    package init(store: StoreOf<ChatFeature>) {
        self.store = store
    }

    package var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if store.availability != .available {
                    Text(store.availability.description)
                        .foregroundStyle(.secondary)
                }
                ForEach(store.conversation.messages) { message in
                    MessageView(message: message, canUndo: store.conversation.changes[message.id] != nil) {
                        store.send(.undoTapped(messageID: message.id))
                    }
                }
                if let replying = store.replying {
                    Text(replying.isEmpty ? "…" : replying)
                        .foregroundStyle(.secondary)
                }
                if let plan = Binding($store.conversation.draft) {
                    DraftCard(plan: plan, problem: store.problem, canAdd: store.canAdd) {
                        store.send(.addTapped)
                    } discard: {
                        store.send(.discardDraftTapped)
                    }
                }
            }
            .padding()
        }
        .defaultScrollAnchor(.bottom)
        .safeAreaInset(edge: .bottom) {
            HStack {
                TextField("Message", text: $store.input, axis: .vertical)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit { store.send(.sendTapped) }
                Button {
                    store.send(.sendTapped)
                } label: {
                    Label("Send", systemImage: "arrow.up.circle.fill")
                        .labelStyle(.iconOnly)
                        .font(.title2)
                }
                .disabled(!store.canSend)
            }
            .padding()
            .background(.bar)
        }
        .navigationTitle(store.conversation.title)
        .task { await store.send(.task).finish() }
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}

private struct MessageView: View {
    let message: ChatMessage
    let canUndo: Bool
    let undo: () -> Void

    var body: some View {
        VStack(alignment: message.role == .person ? .trailing : .leading, spacing: 4) {
            Text(message.text)
                .padding(10)
                .background(
                    message.role == .person ? AnyShapeStyle(.tint.opacity(0.2)) : AnyShapeStyle(.fill.tertiary),
                    in: .rect(cornerRadius: 12))
            if canUndo {
                Button("Undo", action: undo)
                    .font(.caption)
            }
        }
        .frame(maxWidth: .infinity, alignment: message.role == .person ? .trailing : .leading)
    }
}
