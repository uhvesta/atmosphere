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
                Label("Summarized up to here", systemImage: "text.append")
                    .foregroundStyle(.secondary)
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
            // The first lines show; the rest opens.
            Button(isExpanded ? "Show Less" : "Show More") {
                withAnimation(.snappy(duration: 0.25)) { isExpanded.toggle() }
            }
            .buttonStyle(.borderless)
            .fontWeight(.semibold)
        }
        .font(.footnote)
        .padding(10)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 10))
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// Send, or Stop while a reply is being written: a round button in the
/// accent color, gray while there's nothing to send.
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
                Label("Stop", systemImage: "stop.fill")
            }
            .buttonStyle(RoundButtonStyle(prominent: true))
            .help("Stop the reply, keeping what it has written")
            .keyboardShortcut(".", modifiers: .command)
        } else {
            Button(action: send) {
                Label("Send", systemImage: "arrow.up")
            }
            .buttonStyle(RoundButtonStyle(prominent: true))
            .disabled(!canSend)
        }
    }
}

/// A round icon button of the composer: gray, or filled with the accent
/// color when `prominent`, such as Send.
public struct RoundButtonStyle: ButtonStyle {
    let prominent: Bool
    @Environment(\.isEnabled) private var isEnabled

    public init(prominent: Bool = false) {
        self.prominent = prominent
    }

    public func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .labelStyle(.iconOnly)
            .font(.body.weight(.semibold))
            .foregroundStyle(prominent ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
            .frame(width: 34, height: 34)
            .background {
                Circle().fill(fill)
            }
            .opacity(configuration.isPressed ? 0.6 : 1)
            .contentShape(.circle)
    }

    private var fill: AnyShapeStyle {
        guard prominent else { return AnyShapeStyle(.fill.secondary) }
        return isEnabled ? AnyShapeStyle(.tint) : AnyShapeStyle(.gray.opacity(0.4))
    }
}

/// The message field: it grows with what's written, and Return sends. With
/// a skill picked, the field starts with its `SkillToken`, and deleting
/// back past the start of the text removes it.
public struct MessageField: View {
    @Binding var text: String
    let prompt: Text?
    let token: String?
    let removeToken: () -> Void
    let submit: () -> Void

    public init(
        text: Binding<String>, prompt: Text? = nil, token: String? = nil, removeToken: @escaping () -> Void = {},
        submit: @escaping () -> Void
    ) {
        _text = text
        self.prompt = prompt
        self.token = token
        self.removeToken = removeToken
        self.submit = submit
    }

    /// Stands in the field before the text while there's a token, so that
    /// deleting it, which a field can't otherwise tell from an empty one,
    /// removes the token. It takes no room.
    private static let mark = "\u{200B}"

    public var body: some View {
        // The field stays the same view with or without the token, so it
        // keeps the keyboard when typing "/add " turns into one.
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            if let token {
                SkillToken(token)
            }
            TextField("Message", text: field, prompt: token == nil ? prompt : nil, axis: .vertical)
                .textFieldStyle(.plain)
                .lineLimit(1...8)
                .onSubmit(submit)
                .overlay(alignment: .leading) {
                    // The mark keeps the field from showing its prompt.
                    if token != nil, text.isEmpty, let prompt {
                        prompt
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                            .allowsHitTesting(false)
                            .accessibilityHidden(true)
                    }
                }
        }
    }

    private var field: Binding<String> {
        guard token != nil else { return $text }
        return Binding(
            get: { Self.mark + text },
            set: { new in
                if new.contains(Self.mark) {
                    text = new.replacingOccurrences(of: Self.mark, with: "")
                } else {
                    removeToken()
                    text = new
                }
            })
    }
}

/// The model the next message runs on, and how long it thinks, in the
/// composer: tap it to pick another.
public struct ModelPill: View {
    let name: String
    let detail: String?
    let pick: () -> Void

    /// `detail` follows the name in gray, such as the thinking level.
    public init(name: String, detail: String?, pick: @escaping () -> Void) {
        self.name = name
        self.detail = detail
        self.pick = pick
    }

    public var body: some View {
        Button(action: pick) {
            HStack(spacing: 6) {
                Text(name)
                    .foregroundStyle(.primary)
                if let detail {
                    Text(detail)
                        .foregroundStyle(Color.secondary)
                }
            }
            .font(.callout.weight(.medium))
            .lineLimit(1)
            .padding(.horizontal, 12)
            .frame(height: 34)
            .background(.fill.secondary, in: .capsule)
            .contentShape(.capsule)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text("Model"))
        .accessibilityValue(Text(detail.map { "\(name), \($0)" } ?? name))
        .help("Model, Thinking and Context Window")
    }
}

/// The microphone button of the composer: it records for the next
/// message, or for a model that doesn't listen, writes what the person
/// says into the field. Tapped again, it stops.
public struct MicButton: View {
    /// What the microphone does.
    public enum Mode: Hashable, Sendable {
        /// Records a clip to send, for a model that listens.
        case record
        /// Writes what's said into the message field, on the device.
        case dictate
    }

    let isOn: Bool
    let mode: Mode
    let toggle: () -> Void

    /// `isOn` is whether it's recording or dictating now.
    public init(isOn: Bool, mode: Mode, toggle: @escaping () -> Void) {
        self.isOn = isOn
        self.mode = mode
        self.toggle = toggle
    }

    public var body: some View {
        Button(action: toggle) {
            switch (mode, isOn) {
            case (.record, false): Label("Record", systemImage: "mic")
            case (.record, true): Label("Stop Recording", systemImage: "stop.fill")
            case (.dictate, false): Label("Dictate", systemImage: "mic")
            case (.dictate, true): Label("Stop Dictation", systemImage: "stop.fill")
            }
        }
        // Red while it listens, like the system's own.
        .buttonStyle(RoundButtonStyle(prominent: isOn))
        .tint(isOn ? .red : nil)
        .help(help)
    }

    private var help: LocalizedStringKey {
        switch (mode, isOn) {
        case (.record, false): "Record a clip for the model to hear"
        case (.record, true): "Stop and attach the clip"
        case (.dictate, false): "Dictate, on this device"
        case (.dictate, true): "Stop and keep what you said"
        }
    }
}

/// What the next message is written in: a rounded card with what goes
/// with the message on top, such as its skill and attachments, the field,
/// and under it a row with + for what to attach, the model's pill, and
/// on the right the microphone and Send.
public struct ChatComposer<Top: View, Attach: View>: View {
    @Binding var text: String
    let prompt: Text?
    let model: ModelPill?
    let mic: MicButton?
    let send: SendButton
    let submit: () -> Void
    let top: Top
    let attach: Attach
    let plus: Plus
    private var token: String?
    private var removeToken: () -> Void = {}

    /// Starts the field with the skill `name` picked for the message, as
    /// "/name"; deleting it calls `remove`.
    public func token(_ name: String?, remove: @escaping () -> Void) -> Self {
        var composer = self
        composer.token = name
        composer.removeToken = remove
        return composer
    }

    /// What + does.
    enum Plus {
        /// Opens the menu of `attach`.
        case menu
        /// Runs this, such as to open a sheet of what to add.
        case action(() -> Void)
        /// There's no +.
        case none
    }

    /// `attach` is the menu + opens; `top` sits above the field.
    public init(
        text: Binding<String>, prompt: Text? = nil, model: ModelPill?, mic: MicButton? = nil, send: SendButton,
        submit: @escaping () -> Void, @ViewBuilder top: () -> Top, @ViewBuilder attach: () -> Attach
    ) {
        _text = text
        self.prompt = prompt
        self.model = model
        self.mic = mic
        self.send = send
        self.submit = submit
        self.top = top()
        self.attach = attach()
        plus = .menu
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            top
            MessageField(text: $text, prompt: prompt, token: token, removeToken: removeToken, submit: submit)
                .padding(.horizontal, 4)
            HStack(spacing: 8) {
                switch plus {
                case .menu:
                    Menu {
                        attach
                    } label: {
                        Label("Add", systemImage: "plus")
                    }
                    .menuIndicator(.hidden)
                    .buttonStyle(RoundButtonStyle())
                    .fixedSize()
                    .help("Add photos or files")
                case .action(let add):
                    Button(action: add) {
                        Label("Add", systemImage: "plus")
                    }
                    .buttonStyle(RoundButtonStyle())
                    .help("Add photos or files")
                case .none:
                    EmptyView()
                }
                model
                Spacer(minLength: 0)
                mic
                send
            }
        }
        .padding(10)
        .background(.background, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 24, style: .continuous)
                .strokeBorder(.quaternary)
        }
        .shadow(color: .black.opacity(0.06), radius: 8, y: 2)
    }
}

extension ChatComposer where Attach == EmptyView {
    /// A composer whose + runs `add`, such as to open a sheet of what to
    /// add; with no `add`, there's nothing to attach, and so no +.
    public init(
        text: Binding<String>, prompt: Text? = nil, model: ModelPill?, mic: MicButton? = nil, send: SendButton,
        submit: @escaping () -> Void, add: (() -> Void)? = nil, @ViewBuilder top: () -> Top
    ) {
        _text = text
        self.prompt = prompt
        self.model = model
        self.mic = mic
        self.send = send
        self.submit = submit
        self.top = top()
        attach = EmptyView()
        plus = add.map { .action($0) } ?? .none
    }
}

extension ChatComposer where Top == EmptyView, Attach == EmptyView {
    /// A composer with only the field, the model's pill and Send.
    public init(
        text: Binding<String>, prompt: Text? = nil, model: ModelPill?, send: SendButton,
        submit: @escaping () -> Void
    ) {
        self.init(text: text, prompt: prompt, model: model, send: send, submit: submit) { EmptyView() }
    }
}
