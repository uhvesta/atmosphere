import ChatFeatures
import ChatKit
import ChatUI
import ComposableArchitecture
import LangReadrCore
import SwiftUI

/// A chat about a passage: small beside the text, or the whole screen.
struct PassageChatView: View {
    @Bindable var store: StoreOf<PassageChatFeature>

    var body: some View {
        VStack(spacing: 0) {
            Text("“\(store.chat.passage.text)”")
                .font(.headline)
                .lineLimit(store.compact ? 2 : nil)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal)
                .padding(.vertical, 8)
            Divider()
            ChatThreadView(store: store.scope(state: \.thread, action: \.thread), prompt: Text("Ask about it")) {
                Text("Ask what it means, how it's said, or why it's written this way.")
                    .foregroundStyle(.secondary)
            }
        }
        .chatTitle(String(localized: "Chat"), subtitle: store.book.title)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Done") { store.send(.doneTapped) } }
            ToolbarItem(placement: .automatic) {
                // How full the model's window was at the last reply.
                ChatContextButton(store.thread.context)
            }
            if store.compact {
                ToolbarItem(placement: .primaryAction) {
                    // The same chat, taller, with the whole passage.
                    Button("Enlarge") { store.send(.openFullTapped) }
                }
            }
        }
    }
}

/// The chats about a book's passages, the newest first.
struct ChatsView: View {
    let store: StoreOf<ChatsFeature>

    var body: some View {
        List(store.chats) { chat in
            Button {
                store.send(.chatTapped(chat.id))
            } label: {
                VStack(alignment: .leading, spacing: 4) {
                    Text("“\(chat.passage.text)”").font(.headline).lineLimit(1)
                    if let last = chat.conversation.messages.last {
                        Text(last.text).foregroundStyle(.secondary).lineLimit(2)
                    }
                }
            }
            .buttonStyle(.plain)
        }
        .navigationTitle("Chats")
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Done") { store.send(.doneTapped) } }
        }
    }
}
