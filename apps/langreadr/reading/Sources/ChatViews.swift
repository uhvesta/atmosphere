import CaptureKit
import ChatKit
import ChatUI
import ComposableArchitecture
import InferenceKit
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
                prompt: Text("Ask about it"), model: modelPill,
                mic: MicButton(isOn: store.dictating, mode: .dictate) { store.send(.micTapped) },
                add: { store.send(.addContextTapped) },
                pending: store.pending.enumerated().map { index, file in
                    AttachmentChip(verbatim: file.name, systemImage: "doc.text") {
                        store.send(.removeFileTapped(index))
                    }
                },
                files: { $0.files.map(\.name) },
                canSend: store.canSend, canStop: store.reply != nil,
                send: { store.send(.sendTapped) }, stop: { store.send(.stopTapped) },
                empty: {
                    Text("Ask what it means, how it's said, or why it's written this way.")
                        .foregroundStyle(.secondary)
                })
        }
        .chatTitle(store.compact ? String(localized: "Ask") : String(localized: "Chat"), subtitle: store.book.title)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Done") { store.send(.doneTapped) } }
            ToolbarItem(placement: .automatic) {
                // How full the model's window was at the last reply.
                Menu {
                    Text(ContextRing.description(of: store.context))
                } label: {
                    ContextRing(store.context)
                }
                .menuIndicator(.hidden)
                .help(ContextRing.description(of: store.context))
            }
            if store.compact {
                ToolbarItem(placement: .primaryAction) {
                    Button("Open in chat") { store.send(.openFullTapped) }
                }
            }
        }
        .sheet(item: $store.scope(state: \.model, action: \.model)) { ChatModelView(store: $0) }
        .modifier(AddFiles(store: store))
        .task { await store.send(.task).finish() }
    }

    /// Opens what the chat runs on: its model and how long it thinks.
    private var modelPill: ModelPill {
        let current = store.state.current
        return ModelPill(
            name: current?.name ?? String(localized: "Model"),
            detail: current?.thinks == true ? ModelChoice(store.chat.model).thinkingLevel.title : nil,
            pick: { store.send(.modelTapped) })
    }
}

/// + in a chat: on the iPhone a sheet with Add files, on the Mac the file
/// picker itself. Chats read only text, so there are no photos.
private struct AddFiles: ViewModifier {
    @Bindable var store: StoreOf<PassageChatFeature>

    func body(content: Content) -> some View {
        #if os(iOS)
            content.sheet(isPresented: $store.addsContext) {
                AddContextSheet(
                    photos: nil, fileTypes: PickedFile.types(photos: false), close: { store.addsContext = false },
                    tapRecent: { _ in }, photoPicked: { _ in }, photoTaken: { _ in },
                    filesPicked: { store.send(.filesPicked($0)) })
            }
        #else
            content.fileImporter(
                isPresented: $store.addsContext, allowedContentTypes: PickedFile.types(photos: false),
                allowsMultipleSelection: true
            ) { store.send(.filesPicked($0)) }
        #endif
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
