import ChatUI
import ComposableArchitecture
import LifrAssistantClient
import LifrSearchClient
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
                    .sheetLayout(height: 340)
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
            .navigationDestination(isPresented: chatShown()) { chat }
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
                .navigationDestination(isPresented: chatShown(overArchive: false)) { chat }
                .navigationDestination(
                    isPresented: Binding(get: { store.showsArchive }, set: { store.send(.archiveShown($0)) })
                ) {
                    // An archived chat opens over the archive, so Back goes
                    // back to it.
                    ArchivedConversationsView(store: store)
                        .navigationDestination(isPresented: chatShown(overArchive: true)) { chat }
                }
        #endif
    }

    private var list: some View {
        List {
            if store.query.isEmpty {
                ForEach(store.state.sorted) { row(ConversationsFeature.Found(conversation: $0)) }
            } else {
                results
            }
        }
        .searchable(
            text: Binding(get: { store.query }, set: { store.send(.searched($0)) }), prompt: "Search conversations"
        )
        .onSubmit(of: .search) { store.send(.relatedTapped) }
        .overlay {
            if let loadError = store.loadError, store.state.sorted.isEmpty {
                ContentUnavailableView {
                    Label("Couldn't load your conversations", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(loadError)
                } actions: {
                    Button("Try Again") { store.send(.retryTapped) }
                }
            } else if !store.query.isEmpty, foundNothing {
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

    /// A conversation in the list: one a search found, while there's a
    /// search, with the message that has what was searched for.
    private func row(_ found: ConversationsFeature.Found) -> some View {
        let conversation = found.conversation
        return Button {
            store.send(.foundTapped(conversation.id, messageID: found.message?.id))
        } label: {
            ConversationListRow(
                conversation: conversation, replying: store.chats[id: conversation.id]?.core.isReplying == true,
                found: found.message, searched: store.searched.words + store.related)
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

    /// What a search found: conversations with its words, then those
    /// related words found. Words from a model wait for the person to ask.
    @ViewBuilder private var results: some View {
        let found = store.state.found
        let related = found.filter(\.isRelated)
        // A header only when there's something to say: on the Mac, an empty
        // one still takes its space.
        if let during = store.searched.during {
            Section {
                ForEach(found.filter { !$0.isRelated }) { row($0) }
            } header: {
                Text("Written \(during.start..<during.end.addingTimeInterval(-1), format: .interval.month().day())")
            }
        } else {
            Section {
                ForEach(found.filter { !$0.isRelated }) { row($0) }
            }
        }
        if !related.isEmpty {
            Section {
                ForEach(related) { row($0) }
            } header: {
                Text("Related")
            } footer: {
                Text("Also searched for \(store.related.formatted(.list(type: .or)))")
            }
        } else if store.relatedSource == .model, !store.searched.words.isEmpty {
            Section {
                if store.relating {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text("Finding related conversations…")
                    }
                    .foregroundStyle(.secondary)
                    .accessibilityElement(children: .combine)
                } else if store.hasRelated {
                    Text("No related conversations")
                        .foregroundStyle(.secondary)
                } else {
                    Button("Find Related Conversations", systemImage: "sparkles") { store.send(.relatedTapped) }
                }
            }
        }
    }

    /// Whether a search has nothing to show, nor anything left to try.
    private var foundNothing: Bool {
        store.state.found.isEmpty && !store.relating && (store.hasRelated || store.searched.words.isEmpty)
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

    /// Whether a chat is open; with `overArchive`, only while the archive
    /// shows (true) or doesn't (false).
    private func chatShown(overArchive: Bool? = nil) -> Binding<Bool> {
        Binding(
            get: { store.openID != nil && overArchive.map { $0 == store.showsArchive } != false },
            set: { if !$0 { store.send(.closed) } })
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

    /// How to archive one, in the gesture the platform uses for it.
    private var archiveHint: LocalizedStringKey {
        #if os(macOS)
            "Control-click a conversation and choose Archive."
        #else
            "Swipe left on a conversation to archive it."
        #endif
    }

    var body: some View {
        List {
            ForEach(store.state.archivedFound) { conversation in
                // Opens to read; Unarchive there lets it go on.
                Button {
                    store.send(.foundTapped(conversation.id, messageID: nil))
                } label: {
                    ConversationListRow(conversation: conversation, replying: false)
                }
                .buttonStyle(.plain)
                #if os(macOS)
                    .listRowBackground(
                        store.openID == conversation.id
                            ? RoundedRectangle(cornerRadius: 8).fill(.tint.opacity(0.15)) : nil)
                #endif
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
        .searchable(
            text: Binding(get: { store.query }, set: { store.send(.searched($0)) }), prompt: "Search archived"
        )
        .overlay {
            if store.state.archived.isEmpty {
                ContentUnavailableView(
                    "No archived conversations", systemImage: "archivebox",
                    description: Text(archiveHint))
            } else if store.state.archivedFound.isEmpty {
                ContentUnavailableView.search(text: store.query)
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

/// A conversation in the list, with the message a search found, shown from
/// what was searched for, or else the last thing said, to tell
/// conversations apart.
private struct ConversationListRow: View {
    var conversation: Conversation
    var replying: Bool
    /// The message a search found.
    var found: ChatMessage?
    /// The words and phrases searched for.
    var searched: [String] = []

    var body: some View {
        let preview = found ?? conversation.messages.last { !$0.text.isEmpty }
        ConversationRow(
            title: conversation.title, updatedAt: conversation.updatedAt, isReplying: replying,
            preview: found.map { Snippet.around(searched, in: $0.text) } ?? preview?.text,
            previewIsModel: preview?.role == .assistant,
            badge: conversation.draft == nil ? nil : .init("Draft to review", systemImage: "doc.badge.plus"),
            isSynced: conversation.isSynced, searched: found == nil ? [] : searched)
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
