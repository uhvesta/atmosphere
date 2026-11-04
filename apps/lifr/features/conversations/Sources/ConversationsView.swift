import ComposableArchitecture
import SwiftUI

package struct ConversationsView: View {
    @Bindable package var store: StoreOf<ConversationsFeature>

    package init(store: StoreOf<ConversationsFeature>) {
        self.store = store
    }

    package var body: some View {
        List {
            ForEach(store.state.sorted) { conversation in
                Button {
                    store.send(.opened(conversation.id))
                } label: {
                    VStack(alignment: .leading) {
                        Text(conversation.title).lineLimit(1)
                        Text(conversation.updatedAt, format: .dateTime.month().day().hour().minute())
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .swipeActions(allowsFullSwipe: false) {
                    Button("Delete", systemImage: "trash", role: .destructive) {
                        store.send(.deleteTapped(conversation.id))
                    }
                }
                .contextMenu {
                    Button("Delete", systemImage: "trash", role: .destructive) {
                        store.send(.deleteTapped(conversation.id))
                    }
                }
            }
        }
        .overlay {
            if store.conversations.isEmpty {
                ContentUnavailableView {
                    Label("No conversations", systemImage: "bubble.left.and.text.bubble.right")
                } description: {
                    Text("Ask about your tasks, change them, or plan what to add.")
                } actions: {
                    Button("New conversation") { store.send(.newTapped) }
                }
            }
        }
        .navigationTitle("Conversations")
        .task { await store.send(.task).finish() }
        .confirmationDialog($store.scope(state: \.confirm, action: \.confirm))
        .alert($store.scope(state: \.alert, action: \.alert))
        .toolbar {
            ToolbarItem(placement: .secondaryAction) {
                Button("Delete All", systemImage: "trash", role: .destructive) {
                    store.send(.deleteAllTapped)
                }
                .disabled(store.conversations.isEmpty)
            }
            ToolbarItem(placement: .primaryAction) {
                Button {
                    store.send(.newTapped)
                } label: {
                    Label("New conversation", systemImage: "square.and.pencil")
                }
            }
        }
        .navigationDestination(item: $store.scope(state: \.chat, action: \.chat)) { store in
            ChatView(store: store)
        }
    }
}
