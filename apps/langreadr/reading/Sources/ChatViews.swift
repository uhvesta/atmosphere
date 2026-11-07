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
            if let problem = store.problem {
                Text(problem).font(.caption).foregroundStyle(.red).padding(.horizontal)
            }
            Divider()
            CompactChat(
                messages: store.chat.conversation.messages, reply: store.reply, input: $store.input,
                prompt: Text("Ask about it"), canSend: store.canSend, canStop: store.reply != nil,
                send: { store.send(.sendTapped) }, stop: { store.send(.stopTapped) },
                empty: {
                    Text("Ask what it means, how it's said, or why it's written this way.")
                        .foregroundStyle(.secondary)
                })
        }
        .navigationTitle(store.compact ? "Ask" : "Chat")
        #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
        #endif
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Done") { store.send(.doneTapped) } }
            if store.compact {
                ToolbarItem(placement: .primaryAction) {
                    Button("Open in chat") { store.send(.openFullTapped) }
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
