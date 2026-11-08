import ChatFeatures
import ChatUI
import ComposableArchitecture
import SwiftUI

/// LocalLogChatr: the chats beside the one that's open on the Mac, or
/// pushed over them on the iPhone.
package struct ChatsView: View {
    @Bindable var store: StoreOf<ChatsFeature>

    package init(store: StoreOf<ChatsFeature>) {
        self.store = store
    }

    package var body: some View {
        NavigationSplitView {
            list
                .navigationTitle("Chats")
                #if os(macOS)
                    .navigationSplitViewColumnWidth(min: 220, ideal: 260)
                #endif
                .toolbar {
                    ToolbarItem(placement: .primaryAction) {
                        Button("New Chat", systemImage: "square.and.pencil") { store.send(.newTapped) }
                            .keyboardShortcut("n")
                    }
                    ToolbarItem(placement: settingsPlacement) {
                        Button("Settings", systemImage: "gearshape") { store.send(.settingsTapped) }
                    }
                }
        } detail: {
            chat
        }
        .task { await store.send(.task).finish() }
        .sheet(item: $store.scope(state: \.settings, action: \.settings)) { SettingsView(store: $0) }
        .alert("Rename Chat", isPresented: renaming) {
            TextField("Name", text: $store.name)
            Button("Cancel", role: .cancel) { store.send(.renameCancelled) }
            Button("Rename") { store.send(.renameSaved) }
        } message: {
            Text("Leave it empty to name the chat by its first message.")
        }
        .confirmationDialog("Delete this chat?", isPresented: deleting, titleVisibility: .visible) {
            Button("Delete Chat", role: .destructive) { store.send(.deleteConfirmed) }
            Button("Cancel", role: .cancel) { store.send(.deleteCancelled) }
        } message: {
            Text("Its messages are deleted from this device. This can't be undone.")
        }
    }

    private var settingsPlacement: ToolbarItemPlacement {
        #if os(macOS)
            .automatic
        #else
            .topBarLeading
        #endif
    }

    private var list: some View {
        List(selection: Binding(get: { store.openID }, set: { store.send($0.map { .openTapped($0) } ?? .closed) })) {
            if let problem = store.problem {
                Section {
                    Label(problem, systemImage: "exclamationmark.triangle")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    Button("Dismiss") { store.send(.problemDismissed) }
                }
            }
            ForEach(store.state.shown) { chat in
                row(chat)
            }
        }
        .searchable(text: $store.query, prompt: "Search chats")
        .overlay {
            if !store.query.isEmpty, store.state.shown.isEmpty {
                ContentUnavailableView.search(text: store.query)
            } else if store.chats.isEmpty {
                ContentUnavailableView {
                    Label("No chats", systemImage: "bubble.left.and.text.bubble.right")
                } description: {
                    Text("Chat with a model that runs on this device. Nothing you write leaves it.")
                } actions: {
                    Button("New Chat") { store.send(.newTapped) }
                        .buttonStyle(.borderedProminent)
                }
            }
        }
    }

    private func row(_ chat: ChatThreadFeature.State) -> some View {
        let last = chat.thread.messages.last
        return ConversationRow(
            title: chat.thread.title, updatedAt: chat.thread.updatedAt, isReplying: chat.reply != nil,
            preview: last?.text, previewIsModel: last?.role == .assistant, isSynced: false
        )
        .tag(chat.id)
        .swipeActions(edge: .trailing) {
            Button("Delete", systemImage: "trash", role: .destructive) { store.send(.deleteTapped(chat.id)) }
            Button("Rename", systemImage: "pencil") { store.send(.renameTapped(chat.id)) }
                .tint(.gray)
        }
        .contextMenu {
            Button("Rename", systemImage: "pencil") { store.send(.renameTapped(chat.id)) }
            Button("Delete", systemImage: "trash", role: .destructive) { store.send(.deleteTapped(chat.id)) }
        }
    }

    /// The open chat, rebuilt for each so a new one starts with an empty
    /// field.
    @ViewBuilder private var chat: some View {
        if let id = store.openID, let chat = store.scope(state: \.chats[id: id], action: \.chats[id: id]) {
            ChatThreadView(store: chat, prompt: Text("Message")) {
                ChatStart()
            }
            .navigationTitle(chat.thread.title)
            #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    ChatContextButton(chat.context)
                }
            }
            .id(id)
        } else {
            ContentUnavailableView {
                Label("No chat open", systemImage: "bubble.left.and.bubble.right")
            } description: {
                Text("Pick a chat, or start a new one.")
            } actions: {
                Button("New Chat") { store.send(.newTapped) }
            }
        }
    }

    private var renaming: Binding<Bool> {
        Binding(get: { store.renaming != nil }, set: { if !$0 { store.send(.renameCancelled) } })
    }

    private var deleting: Binding<Bool> {
        Binding(get: { store.deleting != nil }, set: { if !$0 { store.send(.deleteCancelled) } })
    }
}

/// What a new chat shows before its first message.
private struct ChatStart: View {
    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: "sparkles")
                .font(.largeTitle)
                .foregroundStyle(.tint)
            Text("Ask anything")
                .font(.title2.weight(.semibold))
            Text("Replies come from the model on this device. Pick another from the model button by the field.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: 360)
        .frame(maxWidth: .infinity)
        .padding(.top, 80)
    }
}
