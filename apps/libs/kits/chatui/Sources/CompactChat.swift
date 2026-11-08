import ChatKit
import InferenceKit
import SwiftUI

/// A small chat to put inside another screen, such as beside what the
/// person is reading: the messages, the reply being written, and the field
/// with the model's pill and Send or Stop. The app keeps the messages and writes the reply; the
/// space it gets is the app's to give.
public struct CompactChat<Message: ChatEntry, Empty: View>: View {
    let messages: [Message]
    let reply: String?
    let thinking: ModelThinking?
    @Binding var input: String
    let prompt: Text?
    let model: ModelPill?
    let canSend: Bool
    let canStop: Bool
    let send: () -> Void
    let stop: () -> Void
    let empty: Empty
    @State private var selecting: TextToSelect?
    @State private var showsThinking = false

    /// `reply` is what's written of the reply so far, nil when none is
    /// being written. `empty` shows before the first message.
    public init(
        messages: [Message], reply: String?, thinking: ModelThinking? = nil, input: Binding<String>,
        prompt: Text? = nil, model: ModelPill? = nil, canSend: Bool, canStop: Bool, send: @escaping () -> Void,
        stop: @escaping () -> Void, @ViewBuilder empty: () -> Empty
    ) {
        self.messages = messages
        self.reply = reply
        self.thinking = thinking
        _input = input
        self.prompt = prompt
        self.model = model
        self.canSend = canSend
        self.canStop = canStop
        self.send = send
        self.stop = stop
        self.empty = empty()
    }

    public var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if messages.isEmpty, reply == nil {
                        empty
                    }
                    ForEach(messages) { message in
                        MessageView(role: message.role, text: message.text) { selecting = TextToSelect(text: $0) }
                            .contextMenu {
                                if !message.text.isEmpty {
                                    TextMenu(text: message.text) { selecting = TextToSelect(text: $0) }
                                }
                            }
                    }
                    if let reply {
                        ReplyInProgress(
                            text: reply, thinking: thinking, showsThinking: showsThinking,
                            toggleThinking: { showsThinking.toggle() })
                    }
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .defaultScrollAnchor(.bottom)
            ChatComposer(
                text: $input, prompt: prompt, model: model,
                send: SendButton(canSend: canSend, canStop: canStop, send: send, stop: stop),
                submit: { if canSend { send() } }
            )
            .padding(10)
        }
        .sheet(item: $selecting) { SelectTextSheet(text: $0.text) }
    }
}

extension CompactChat where Empty == EmptyView {
    public init(
        messages: [Message], reply: String?, thinking: ModelThinking? = nil, input: Binding<String>,
        prompt: Text? = nil, model: ModelPill? = nil, canSend: Bool, canStop: Bool, send: @escaping () -> Void,
        stop: @escaping () -> Void
    ) {
        self.init(
            messages: messages, reply: reply, thinking: thinking, input: input, prompt: prompt, model: model,
            canSend: canSend,
            canStop: canStop, send: send, stop: stop
        ) { EmptyView() }
    }
}
