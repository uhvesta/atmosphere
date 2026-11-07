import ChatKit
import Foundation

// A small chat to test conversations on: plain messages, and a context
// that remembers which replies saved something.

struct Message: ChatEntry {
    var id: UUID
    var role: ChatRole
    var text: String
    var at: Date
}

struct Saved: ConversationContext {
    var saved: [UUID: Int] = [:]

    init() {}

    init(from decoder: any Decoder) throws {
        saved = try decoder.container(keyedBy: CodingKeys.self).decodeIfPresent([UUID: Int].self, forKey: .saved) ?? [:]
    }
}

typealias Chat = Conversation<Message, Saved>

let start = Date(timeIntervalSince1970: 1_793_000_000)

/// A conversation of person and assistant turns, a minute apart.
func chat(_ texts: [String]) -> Chat {
    var chat = Chat(id: UUID(), createdAt: start)
    for (index, text) in texts.enumerated() {
        chat.append(
            Message(
                id: UUID(), role: index.isMultiple(of: 2) ? .person : .assistant, text: text,
                at: start + Double(index) * 60))
    }
    return chat
}

extension Chat {
    func id(_ text: String) throws -> UUID {
        try (messages + hidden).first { $0.text == text }.unwrap("no message \(text)").id
    }
}
