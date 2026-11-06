import ComposableArchitecture
import LifrAssistantClient
import LifrUI
import SwiftUI

/// The conversations not archived. Swipe right to sync one or stop syncing
/// it; swipe left to archive it. The archive is the one place to delete.
package struct ConversationsView: View {
    @Bindable package var store: StoreOf<ConversationsFeature>

    package init(store: StoreOf<ConversationsFeature>) {
        self.store = store
    }

    package var body: some View {
        screen
            .task { await store.send(.task).finish() }
            .confirmationDialog($store.scope(state: \.confirm, action: \.confirm))
            .alert($store.scope(state: \.alert, action: \.alert))
            .sheet(item: $store.scope(state: \.rename, action: \.rename)) { store in
                RenameView(store: store)
                    .sheetLayout(height: 200)
            }
    }

    @ViewBuilder private var screen: some View {
        #if os(macOS)
            // The Mac shows the list beside the chat, so the archive is a
            // switch over the list rather than a screen of its own.
            Group {
                if store.showsArchive {
                    ArchivedConversationsView(store: store)
                } else {
                    list
                }
            }
            .toolbar {
                ToolbarItem(placement: .principal) {
                    Picker(
                        "Show",
                        selection: Binding(get: { store.showsArchive }, set: { store.send(.archiveShown($0)) })
                    ) {
                        Text("Conversations").tag(false)
                        Text("Archived").tag(true)
                    }
                    .pickerStyle(.segmented)
                }
                newButton
            }
            .navigationDestination(isPresented: chatShown) { chat }
        #else
            list
                .toolbar {
                    ToolbarItem(placement: .secondaryAction) {
                        Button("Archived", systemImage: "archivebox") {
                            store.send(.archiveShown(true))
                        }
                    }
                    newButton
                }
                .navigationDestination(isPresented: chatShown) { chat }
                .navigationDestination(
                    isPresented: Binding(get: { store.showsArchive }, set: { store.send(.archiveShown($0)) })
                ) {
                    ArchivedConversationsView(store: store)
                }
        #endif
    }

    /// The conversations shown: those that have what's searched for, and
    /// where, while there's a search.
    private var shown: [ConversationsFeature.Found] {
        store.query.isEmpty ? store.state.sorted.map { .init(conversation: $0) } : store.state.found
    }

    private var list: some View {
        List {
            ForEach(shown) { found in
                let conversation = found.conversation
                Button {
                    store.send(.foundTapped(conversation.id, messageID: found.message?.id))
                } label: {
                    ConversationRow(
                        conversation: conversation, replying: store.chats[id: conversation.id]?.replying != nil,
                        found: found.message, query: store.query)
                }
                .buttonStyle(.plain)
                #if os(macOS)
                    .listRowBackground(
                        store.openID == conversation.id
                            ? RoundedRectangle(cornerRadius: 8).fill(.tint.opacity(0.15)) : nil)
                #endif
                .swipeActions(edge: .leading, allowsFullSwipe: false) {
                    SyncButton(conversation: conversation, store: store).tint(.blue)
                }
                .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                    ArchiveButton(id: conversation.id, store: store).tint(.indigo)
                    RenameButton(id: conversation.id, store: store).tint(.gray)
                }
                .contextMenu {
                    RenameButton(id: conversation.id, store: store)
                    SyncButton(conversation: conversation, store: store)
                    ArchiveButton(id: conversation.id, store: store)
                }
            }
        }
        .searchable(
            text: Binding(get: { store.query }, set: { store.send(.searched($0)) }), prompt: "Search conversations"
        )
        .overlay {
            if !store.query.isEmpty, shown.isEmpty {
                ContentUnavailableView.search(text: store.query)
            } else if store.state.sorted.isEmpty {
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
    }

    private var newButton: some ToolbarContent {
        ToolbarItem(placement: .primaryAction) {
            Button {
                store.send(.newTapped)
            } label: {
                Label("New conversation", systemImage: "square.and.pencil")
            }
        }
    }

    private var chatShown: Binding<Bool> {
        Binding(get: { store.openID != nil }, set: { if !$0 { store.send(.closed) } })
    }

    /// The open chat, rebuilt for each conversation so a new one starts
    /// with an empty field at the top.
    @ViewBuilder private var chat: some View {
        if let id = store.openID, let chat = store.scope(state: \.chats[id: id], action: \.chats[id: id]) {
            ChatView(store: chat).id(id)
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
    /// The message a search found, shown from what was searched for.
    var found: ChatMessage?
    var query = ""

    /// The message a search found, or else the last thing said, to tell
    /// conversations apart.
    private var preview: ChatMessage? {
        found ?? conversation.messages.last { !$0.text.isEmpty }
    }

    /// `preview`'s text, from a little before what was searched for when
    /// that's further in.
    private var previewText: String {
        guard let preview else { return "" }
        let text = preview.text
        guard found != nil,
            let range = text.range(of: query, options: [.caseInsensitive, .diacriticInsensitive]),
            text.distance(from: text.startIndex, to: range.lowerBound) > 40
        else { return text }
        return "…" + text[text.index(range.lowerBound, offsetBy: -30)...]
    }

    var body: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline) {
                    Text(conversation.title)
                        .font(.headline)
                        .lineLimit(1)
                    Spacer()
                    Text(conversation.updatedAt, format: .dateTime.month().day().hour().minute())
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                if replying {
                    Text("Replying…")
                        .font(.subheadline)
                        .foregroundStyle(.tint)
                } else if let preview {
                    Text(previewText)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .accessibilityIdentifier(preview.role == .assistant ? ModelText.identifier : "")
                }
                if conversation.draft != nil {
                    Label("Draft to review", systemImage: "doc.badge.plus")
                        .font(.caption)
                        .foregroundStyle(.tint)
                }
            }
            if conversation.isSynced {
                Image(systemName: "icloud")
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Synced")
            }
        }
        .padding(.vertical, 4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(.rect)
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

private struct RenameButton: View {
    var id: UUID
    var store: StoreOf<ConversationsFeature>

    var body: some View {
        Button("Rename", systemImage: "pencil") { store.send(.renameTapped(id)) }
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
