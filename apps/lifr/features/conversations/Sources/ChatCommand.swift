import Foundation
import LifrAssistantClient

/// What typing "/name" in the chat does besides picking a skill: each runs
/// the moment it's picked from the / list, or typed and followed by a
/// space or Return. None of them loses anything: going back keeps what
/// came after, and a summary can be forgotten.
package enum ChatCommand: String, CaseIterable, Equatable, Sendable {
    case compact
    case find
    case back
    case forward
    case checkpoints
    case rename
    case model
    case skills
    case new
    case export
    case paste

    /// What follows the / to run it.
    package var name: String { rawValue }

    /// What it does, in the / list.
    package var summary: String {
        switch self {
        case .compact: String(localized: "Summarize older messages, so the assistant keeps them in mind.")
        case .find: String(localized: "Find words in this conversation.")
        case .back: String(localized: "Go back to the message before. What came after is kept.")
        case .forward: String(localized: "Go forward to the message after.")
        case .checkpoints: String(localized: "Every point to go back or forward to.")
        case .rename: String(localized: "Rename this conversation.")
        case .model: String(localized: "Pick the model, thinking and context window.")
        case .skills: String(localized: "Turn skills on or off, or write your own.")
        case .new: String(localized: "Start a new conversation.")
        case .export: String(localized: "Copy a prompt to plan with another chat assistant.")
        case .paste: String(localized: "Review the plan in another assistant's reply, copied to the clipboard.")
        }
    }

    /// Whether it can run in the chat now: it's only listed, and only
    /// runs, when it can.
    func isAvailable(in state: ChatFeature.State) -> Bool {
        switch self {
        case .compact: state.canCompact
        case .find, .checkpoints: !state.conversation.messages.isEmpty
        case .back: state.canGo && state.conversation.back != nil
        case .forward: state.canGo && state.conversation.forward != nil
        case .rename, .model, .skills, .new, .export: true
        case .paste: state.core.reply == nil && !state.editsDraft
        }
    }
}

/// One line of the / list: a skill to pick for the next message, or a
/// command to run.
package enum SlashChoice: Equatable, Identifiable {
    case skill(Skill)
    case command(ChatCommand)

    /// What's typed after the / for it, such as "tidy" or "compact".
    package var id: String {
        switch self {
        case .skill(let skill): skill.id
        case .command(let command): command.name
        }
    }

    package var summary: String {
        switch self {
        case .skill(let skill): skill.summary
        case .command(let command): command.summary
        }
    }
}

/// What the message field starts with when it's "/name" and then a space,
/// a new line or the end: the name and what follows it.
struct TypedSlash: Equatable {
    var name: String
    /// What follows the space or new line after the name.
    var details: String

    /// The slash at the start of `text`. Without a space or new line after
    /// the name, only when `complete`, as on Send: until then the name may
    /// still be the start of a longer one.
    init?(_ text: String, complete: Bool) {
        guard text.hasPrefix("/") else { return nil }
        let rest = text.dropFirst()
        if let end = rest.firstIndex(where: { $0 == " " || $0.isNewline }) {
            name = String(rest[..<end])
            details = String(rest[rest.index(after: end)...])
        } else {
            guard complete else { return nil }
            name = String(rest)
            details = ""
        }
        guard !name.isEmpty else { return nil }
    }
}
