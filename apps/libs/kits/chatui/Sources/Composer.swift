import SwiftUI

/// Finds words in the messages shown, one match at a time: the field, how
/// many matched and which is shown, and buttons to the one before or after.
public struct FindBar: View {
    @Binding var query: String
    let matches: Int
    let position: Int
    let move: (Int) -> Void
    let close: () -> Void
    @FocusState private var focused: Bool

    /// `position` is the match shown, from 0; `move` gets -1 or 1.
    public init(
        query: Binding<String>, matches: Int, position: Int, move: @escaping (Int) -> Void,
        close: @escaping () -> Void
    ) {
        _query = query
        self.matches = matches
        self.position = position
        self.move = move
        self.close = close
    }

    public var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
                .accessibilityHidden(true)
            TextField("Find in conversation", text: $query)
                .accessibilityLabel("Find in conversation")
                .focused($focused)
                .onSubmit { move(-1) }
                .onAppear { focused = true }
            if !query.isEmpty {
                Text(matches == 0 ? "No matches" : "\(position + 1) of \(matches)")
                    .font(.footnote.monospacedDigit())
                    .foregroundStyle(.secondary)
                Button("Previous Match", systemImage: "chevron.up") { move(-1) }
                    .disabled(matches < 2)
                Button("Next Match", systemImage: "chevron.down") { move(1) }
                    .disabled(matches < 2)
            }
            Button("Done", action: close)
                .font(.body.weight(.semibold))
                .keyboardShortcut(.cancelAction)
        }
        .labelStyle(.iconOnly)
        .buttonStyle(.borderless)
    }
}

/// A line above the message field on the state of the chat, with what to
/// do about it, or a spinner while that's under way.
public struct ChatNotice: View {
    let text: LocalizedStringKey
    let systemImage: String
    let action: LocalizedStringKey?
    let isBusy: Bool
    let isEnabled: Bool
    let perform: () -> Void

    public init(
        _ text: LocalizedStringKey, systemImage: String, action: LocalizedStringKey? = nil, isBusy: Bool = false,
        isEnabled: Bool = true, perform: @escaping () -> Void = {}
    ) {
        self.text = text
        self.systemImage = systemImage
        self.action = action
        self.isBusy = isBusy
        self.isEnabled = isEnabled
        self.perform = perform
    }

    public var body: some View {
        HStack {
            if isBusy {
                ProgressView().controlSize(.small)
                Text(text)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                Spacer()
            } else {
                Label(text, systemImage: systemImage)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                Spacer()
                if let action {
                    Button(action, action: perform)
                        .font(.footnote.weight(.semibold))
                        .disabled(!isEnabled)
                }
            }
        }
        .padding(.horizontal)
        .padding(.vertical, 8)
    }
}

/// Under the last message a summary covers: what the model gets in place of
/// the messages up to here, and Forget to send them again instead.
public struct SummaryMarker: View {
    let text: String
    let canForget: Bool
    let forget: () -> Void

    @State private var isExpanded = false

    public init(text: String, canForget: Bool, forget: @escaping () -> Void) {
        self.text = text
        self.canForget = canForget
        self.forget = forget
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Button {
                    withAnimation { isExpanded.toggle() }
                } label: {
                    Label("Summarized up to here", systemImage: isExpanded ? "chevron.down" : "chevron.right")
                }
                .accessibilityValue(isExpanded ? Text("Shown") : Text("Hidden"))
                Spacer()
                Button("Forget Summary", role: .destructive, action: forget)
                    .disabled(!canForget)
                    .help("Send the messages above again, as far as they fit")
            }
            .buttonStyle(.borderless)
            Text(text)
                .lineLimit(isExpanded ? nil : 2)
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
        }
        .font(.footnote)
        .padding(10)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 10))
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// Send, or Stop while a reply is being written.
public struct SendButton: View {
    let canSend: Bool
    let canStop: Bool
    let send: () -> Void
    let stop: () -> Void

    public init(canSend: Bool, canStop: Bool, send: @escaping () -> Void, stop: @escaping () -> Void) {
        self.canSend = canSend
        self.canStop = canStop
        self.send = send
        self.stop = stop
    }

    public var body: some View {
        if canStop {
            Button(action: stop) {
                Label("Stop", systemImage: "stop.circle.fill")
                    .labelStyle(.iconOnly)
                    .font(.title2)
            }
            .help("Stop the reply, keeping what it has written")
            .keyboardShortcut(".", modifiers: .command)
        } else {
            Button(action: send) {
                Label("Send", systemImage: "arrow.up.circle.fill")
                    .labelStyle(.iconOnly)
                    .font(.title2)
            }
            .disabled(!canSend)
        }
    }
}

/// The message field: it grows with what's written, and Return sends.
public struct MessageField: View {
    @Binding var text: String
    let prompt: Text?
    let submit: () -> Void

    public init(text: Binding<String>, prompt: Text? = nil, submit: @escaping () -> Void) {
        _text = text
        self.prompt = prompt
        self.submit = submit
    }

    public var body: some View {
        TextField("Message", text: $text, prompt: prompt, axis: .vertical)
            .textFieldStyle(.roundedBorder)
            .onSubmit(submit)
    }
}
