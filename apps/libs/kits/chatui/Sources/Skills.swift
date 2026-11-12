import AssistantKit
import SwiftUI

/// The skill a message is for, as its "/name" at the start of the text:
/// monospaced in the accent color, on a tinted rounded background, like
/// code in a line of text.
public struct SkillToken: View {
    let name: String

    /// `name` is what follows the /, such as "add".
    public init(_ name: String) {
        self.name = name
    }

    public var body: some View {
        Text(verbatim: "/\(name)")
            .font(.body.monospaced())
            .foregroundStyle(.tint)
            .lineLimit(1)
            .fixedSize()
            .padding(.horizontal, 5)
            .padding(.vertical, 1)
            .background(.tint.opacity(0.14), in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(Text("Skill: \(name)"))
    }
}

/// A skill to turn on or off, with what it does and whether it's picked
/// automatically.
public struct SkillToggle<Tool: SkillTool>: View {
    let skill: Skill<Tool>
    let toggle: (Bool) -> Void

    public init(_ skill: Skill<Tool>, toggle: @escaping (Bool) -> Void) {
        self.skill = skill
        self.toggle = toggle
    }

    public var body: some View {
        Toggle(isOn: Binding(get: { skill.isOn }, set: toggle)) {
            VStack(alignment: .leading, spacing: 2) {
                Text(skill.title)
                Text(skill.autoPicks ? "\(skill.summary) Automatic." : skill.summary)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

/// One line of the / list: what's typed after the / for it, and what it
/// does.
public struct SlashSuggestion: Identifiable, Hashable, Sendable {
    /// What follows the /, such as "add".
    public var id: String
    public var summary: String

    public init(_ name: String, summary: String) {
        id = name
        self.summary = summary
    }
}

/// The skills and commands matching what's typed after a / in the message
/// field, in a card above it that scrolls. Picking a skill makes it the
/// message's; picking a command runs it.
public struct SlashSuggestions: View {
    let suggestions: [SlashSuggestion]
    let pick: (String) -> Void
    /// About one row with a line of summary, at the person's text size.
    @ScaledMetric(relativeTo: .body) private var rowHeight: CGFloat = 60

    public init(_ suggestions: [SlashSuggestion], pick: @escaping (String) -> Void) {
        self.suggestions = suggestions
        self.pick = pick
    }

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(suggestions) { suggestion in
                    Button {
                        pick(suggestion.id)
                    } label: {
                        VStack(alignment: .leading, spacing: 2) {
                            // What's typed to pick it: "/query".
                            Text(verbatim: "/\(suggestion.id)")
                                .font(.body.monospaced())
                            if !suggestion.summary.isEmpty {
                                Text(suggestion.summary)
                                    .font(.caption)
                                    .foregroundStyle(Color.secondary)
                                    .lineLimit(2)
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 10)
                        .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    if suggestion.id != suggestions.last?.id {
                        Divider().padding(.leading, 14)
                    }
                }
            }
        }
        .scrollBounceBehavior(.basedOnSize)
        // Shows there's more below when not all fit.
        .scrollIndicatorsFlash(onAppear: true)
        // Four and a half rows, so a cut row says there's more to scroll to.
        .frame(maxHeight: rowHeight * 4.5)
        .fixedSize(horizontal: false, vertical: true)
        // The shadow goes on the card alone: on the scroll view, it would be
        // drawn under each row and wash out their gray text.
        .background {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(.background)
                .shadow(color: .black.opacity(0.08), radius: 10, y: 2)
        }
        .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous).strokeBorder(.quaternary)
        }
    }
}
