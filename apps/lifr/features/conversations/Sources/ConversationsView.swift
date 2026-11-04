import ComposableArchitecture
import LifrAssistantClient
import SwiftUI

/// The conversations not archived. Swipe right to sync one or stop syncing
/// it; swipe left to archive it. The archive is the one place to delete.
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
                    ConversationRow(
                        conversation: conversation, replying: store.chats[id: conversation.id]?.replying != nil)
                }
                .swipeActions(edge: .leading, allowsFullSwipe: false) {
                    SyncButton(conversation: conversation, store: store).tint(.blue)
                }
                .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                    ArchiveButton(id: conversation.id, store: store).tint(.indigo)
                }
                .contextMenu {
                    SyncButton(conversation: conversation, store: store)
                    ArchiveButton(id: conversation.id, store: store)
                }
            }
        }
        .overlay {
            if store.state.sorted.isEmpty {
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
                Button("Archived", systemImage: "archivebox") {
                    store.send(.archiveShown(true))
                }
            }
            ToolbarItem(placement: .primaryAction) {
                Button {
                    store.send(.newTapped)
                } label: {
                    Label("New conversation", systemImage: "square.and.pencil")
                }
            }
        }
        .navigationDestination(
            isPresented: Binding(get: { store.openID != nil }, set: { if !$0 { store.send(.closed) } })
        ) {
            if let id = store.openID, let chat = store.scope(state: \.chats[id: id], action: \.chats[id: id]) {
                ChatView(store: chat)
            }
        }
        .navigationDestination(
            isPresented: Binding(get: { store.showsArchive }, set: { store.send(.archiveShown($0)) })
        ) {
            ArchivedConversationsView(store: store)
        }
    }
}

/// Archived conversations, to put back or delete for good.
struct ArchivedConversationsView: View {
    @Bindable var store: StoreOf<ConversationsFeature>

    var body: some View {
        List {
            ForEach(store.state.archived) { conversation in
                ConversationRow(conversation: conversation, replying: false)
                    .swipeActions(edge: .leading, allowsFullSwipe: false) {
                        UnarchiveButton(id: conversation.id, store: store).tint(.indigo)
                    }
                    .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                        DeleteButton(id: conversation.id, store: store)
                    }
                    .contextMenu {
                        UnarchiveButton(id: conversation.id, store: store)
                        DeleteButton(id: conversation.id, store: store)
                    }
            }
        }
        .overlay {
            if store.state.archived.isEmpty {
                ContentUnavailableView(
                    "No archived conversations", systemImage: "archivebox",
                    description: Text("Swipe left on a conversation to archive it."))
            }
        }
        .navigationTitle("Archived")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Delete All", systemImage: "trash", role: .destructive) {
                    store.send(.deleteArchivedTapped)
                }
                .disabled(store.state.archived.isEmpty)
            }
        }
    }
}

private struct ConversationRow: View {
    var conversation: Conversation
    var replying: Bool

    var body: some View {
        HStack {
            VStack(alignment: .leading) {
                Text(conversation.title).lineLimit(1)
                if replying {
                    Text("Replying…")
                        .font(.caption)
                        .foregroundStyle(.tint)
                } else {
                    Text(conversation.updatedAt, format: .dateTime.month().day().hour().minute())
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            if conversation.isSynced {
                Spacer()
                Image(systemName: "icloud")
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Synced")
            }
        }
    }
}

private struct SyncButton: View {
    var conversation: Conversation
    var store: StoreOf<ConversationsFeature>

    var body: some View {
        if conversation.isSynced {
            Button("Stop Syncing", systemImage: "icloud.slash") { store.send(.syncTapped(conversation.id)) }
        } else {
            Button("Sync", systemImage: "icloud") { store.send(.syncTapped(conversation.id)) }
        }
    }
}

private struct ArchiveButton: View {
    var id: UUID
    var store: StoreOf<ConversationsFeature>

    var body: some View {
        Button("Archive", systemImage: "archivebox") { store.send(.archiveTapped(id)) }
    }
}

private struct UnarchiveButton: View {
    var id: UUID
    var store: StoreOf<ConversationsFeature>

    var body: some View {
        Button("Unarchive", systemImage: "tray.and.arrow.up") { store.send(.unarchiveTapped(id)) }
    }
}

private struct DeleteButton: View {
    var id: UUID
    var store: StoreOf<ConversationsFeature>

    var body: some View {
        Button("Delete", systemImage: "trash", role: .destructive) { store.send(.deleteTapped(id)) }
    }
}
