import ChatKit
import InferenceKit
import SwiftUI

/// One message: the person's in a bubble on the trailing side, the
/// assistant's as text in the Markdown it wrote, with what the model thought, the skill it
/// was written for, what's attached and Undo for what it changed.
public struct MessageView<Attachments: View>: View {
    let role: ChatRole
    let text: String
    let thinking: ModelThinking?
    /// The skill's name, after the /: it starts the person's text.
    let skill: String?
    let canUndo: Bool
    let showsThinking: Bool
    let toggleThinking: () -> Void
    let undo: () -> Void
    /// Opens text to select part of.
    let select: (String) -> Void
    let attachments: Attachments

    public init(
        role: ChatRole, text: String, thinking: ModelThinking? = nil, skill: String? = nil, canUndo: Bool = false,
        showsThinking: Bool = false, toggleThinking: @escaping () -> Void = {}, undo: @escaping () -> Void = {},
        select: @escaping (String) -> Void, @ViewBuilder attachments: () -> Attachments
    ) {
        self.role = role
        self.text = text
        self.thinking = thinking
        self.skill = skill
        self.canUndo = canUndo
        self.showsThinking = showsThinking
        self.toggleThinking = toggleThinking
        self.undo = undo
        self.select = select
        self.attachments = attachments()
    }

    public var body: some View {
        VStack(alignment: role == .person ? .trailing : .leading, spacing: 4) {
            if let thinking, role == .assistant {
                ThinkingDisclosure(
                    thinking: thinking, isExpanded: showsThinking, toggle: toggleThinking, select: select)
            }
            if let skill, role == .assistant {
                Label(skill, systemImage: "sparkles")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.tint)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(Text("Skill: \(skill)"))
            }
            attachments
            if !text.isEmpty || (role == .person && skill != nil) {
                if role == .person {
                    // The skill starts the text, as it did in the field.
                    HStack(alignment: .firstTextBaseline, spacing: 4) {
                        if let skill { SkillToken(skill) }
                        if !text.isEmpty {
                            // Marks such as **bold** read as they do in a reply.
                            Text(InlineText.styled(text))
                                .selectableOnMac()
                        }
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(.tint.opacity(0.2), in: .rect(cornerRadius: 16))
                    .frame(maxWidth: 520, alignment: .trailing)
                } else {
                    // The assistant's words read as text, not a bubble,
                    // in the Markdown it writes.
                    MarkdownText(text: text)
                        .selectableOnMac()
                }
            }
            if canUndo {
                Button("Undo", action: undo)
                    .font(.caption)
            }
        }
        .frame(maxWidth: .infinity, alignment: role == .person ? .trailing : .leading)
    }
}

extension MessageView where Attachments == EmptyView {
    public init(
        role: ChatRole, text: String, thinking: ModelThinking? = nil, skill: String? = nil, canUndo: Bool = false,
        showsThinking: Bool = false, toggleThinking: @escaping () -> Void = {}, undo: @escaping () -> Void = {},
        select: @escaping (String) -> Void
    ) {
        self.init(
            role: role, text: text, thinking: thinking, skill: skill, canUndo: canUndo,
            showsThinking: showsThinking, toggleThinking: toggleThinking, undo: undo, select: select
        ) { EmptyView() }
    }
}

/// What the model thought before it replied, behind a chevron: closed, a
/// line saying how long it thought and on what; open, the thoughts too.
public struct ThinkingDisclosure: View {
    let thinking: ModelThinking
    let isExpanded: Bool
    let toggle: () -> Void
    /// Opens the thoughts to select part of; nil while they're written.
    var select: ((String) -> Void)?

    public init(
        thinking: ModelThinking, isExpanded: Bool, toggle: @escaping () -> Void, select: ((String) -> Void)? = nil
    ) {
        self.thinking = thinking
        self.isExpanded = isExpanded
        self.toggle = toggle
        self.select = select
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button(action: toggle) {
                HStack(spacing: 6) {
                    Image(systemName: "chevron.right")
                        .font(.caption.weight(.semibold))
                        .rotationEffect(.degrees(isExpanded ? 90 : 0))
                    Text(summary)
                    if thinking.duration == nil { ProgressView().controlSize(.mini) }
                }
                .font(.footnote)
                .foregroundStyle(.secondary)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(summary)
            .accessibilityHint(isExpanded ? Text("Hides the thinking") : Text("Shows the thinking"))
            .accessibilityAddTraits(isExpanded ? .isSelected : [])
            if isExpanded {
                Text(thoughts)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .selectableOnMac()
                    .contextMenu {
                        if let select { TextMenu(text: thoughts, select: select) }
                    }
                    .padding(.leading, 10)
                    .overlay(alignment: .leading) {
                        Rectangle().fill(.quaternary).frame(width: 2)
                    }
            }
        }
        .animation(.snappy(duration: 0.2), value: isExpanded)
    }

    private var thoughts: String { thinking.text.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var summary: String {
        guard let duration = thinking.duration else { return String(localized: "Thinking…") }
        let time = duration.formatted(
            .units(allowed: [.seconds], width: .abbreviated, fractionalPart: .show(length: 1)))
        switch thinking.processor {
        case .gpu?: return String(localized: "Thoughts · \(time) on the GPU")
        case .cpu?: return String(localized: "Thoughts · \(time) on the CPU")
        case nil: return String(localized: "Thoughts · \(time)")
        }
    }
}

/// A reply being written: what the model thinks, then its words, or
/// "Thinking…" until either comes.
public struct ReplyInProgress: View {
    let text: String
    let thinking: ModelThinking?
    let showsThinking: Bool
    let toggleThinking: () -> Void

    public init(
        text: String, thinking: ModelThinking?, showsThinking: Bool, toggleThinking: @escaping () -> Void
    ) {
        self.text = text
        self.thinking = thinking
        self.showsThinking = showsThinking
        self.toggleThinking = toggleThinking
    }

    public var body: some View {
        if let thinking {
            ThinkingDisclosure(thinking: thinking, isExpanded: showsThinking, toggle: toggleThinking)
        }
        if text.isEmpty, thinking == nil {
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text("Thinking…")
            }
            .foregroundStyle(.secondary)
            .accessibilityElement(children: .combine)
        } else {
            MarkdownText(text: text, isWriting: true).foregroundStyle(.secondary)
        }
    }
}

/// ‹ 2/3 › under a message or reply written again, to show the others.
public struct BranchSwitcher: View {
    let index: Int
    let count: Int
    let canGo: Bool
    let move: (Int) -> Void

    /// `index` of `count` versions; `move` gets -1 or 1.
    public init(index: Int, count: Int, canGo: Bool, move: @escaping (Int) -> Void) {
        self.index = index
        self.count = count
        self.canGo = canGo
        self.move = move
    }

    public var body: some View {
        HStack(spacing: 4) {
            Button("Previous Version", systemImage: "chevron.left") { move(-1) }
                .disabled(index == 0 || !canGo)
            Text("\(index + 1)/\(count)")
                .monospacedDigit()
                .accessibilityLabel(Text("Version \(index + 1) of \(count)"))
            Button("Next Version", systemImage: "chevron.right") { move(1) }
                .disabled(index == count - 1 || !canGo)
        }
        .labelStyle(.iconOnly)
        .buttonStyle(.borderless)
        .font(.caption)
        .foregroundStyle(.secondary)
    }
}

/// A photo, recording or file: on the next message with a button to remove
/// it, or on a sent one, saying when its file is on another device. With
/// `open`, tapping it opens what's attached.
public struct AttachmentChip: View {
    let title: Text
    let systemImage: String
    let elsewhere: Bool
    var remove: (() -> Void)?
    var open: (() -> Void)?

    public init(
        _ title: LocalizedStringKey, systemImage: String, elsewhere: Bool = false, remove: (() -> Void)? = nil
    ) {
        self.title = Text(title)
        self.systemImage = systemImage
        self.elsewhere = elsewhere
        self.remove = remove
    }

    /// A chip titled as is, such as with a file's name.
    public init(
        verbatim title: String, systemImage: String, elsewhere: Bool = false, open: (() -> Void)? = nil,
        remove: (() -> Void)? = nil
    ) {
        self.title = Text(verbatim: title)
        self.systemImage = systemImage
        self.elsewhere = elsewhere
        self.open = open
        self.remove = remove
    }

    public var body: some View {
        HStack(spacing: 6) {
            if let open {
                Button(action: open) { label }
                    .buttonStyle(.plain)
                    .accessibilityHint(Text("Opens it"))
            } else {
                label
            }
            if let remove {
                Button("Remove", systemImage: "xmark.circle.fill", action: remove)
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
                    .tint(.secondary)
            }
        }
        .font(.callout)
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(Color.secondary.opacity(0.15), in: .capsule)
        .accessibilityElement(children: open == nil ? .combine : .contain)
    }

    private var label: some View {
        HStack(spacing: 6) {
            Label {
                title.lineLimit(1)
            } icon: {
                Image(systemName: systemImage)
            }
            if elsewhere {
                Text("Not on this device").foregroundStyle(.secondary)
            }
        }
        .contentShape(.rect)
    }
}

/// Ways to start, in a chat with nothing in it yet: a line on what to ask,
/// and a few messages to send with a tap.
public struct Suggestions: View {
    public struct Suggestion: Hashable, Sendable {
        public var systemImage: String
        public var text: String

        public init(systemImage: String, text: String) {
            self.systemImage = systemImage
            self.text = text
        }
    }

    let intro: LocalizedStringKey
    let suggestions: [Suggestion]
    let send: (String) -> Void

    public init(_ intro: LocalizedStringKey, suggestions: [Suggestion], send: @escaping (String) -> Void) {
        self.intro = intro
        self.suggestions = suggestions
        self.send = send
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(intro)
                .foregroundStyle(.secondary)
            ForEach(suggestions, id: \.text) { suggestion in
                Button {
                    send(suggestion.text)
                } label: {
                    Label(suggestion.text, systemImage: suggestion.systemImage)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 10)
                        .background(.fill.quaternary, in: .rect(cornerRadius: 12))
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
            }
        }
    }
}
