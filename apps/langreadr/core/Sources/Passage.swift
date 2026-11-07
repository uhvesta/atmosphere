import ChatKit
import Foundation

/// Part of a paragraph the person highlighted, from one word to another.
public struct Passage: Hashable, Codable, Sendable {
    public var paragraph: UUID
    /// Where it starts and ends in the paragraph's text, in characters.
    public var start: Int
    public var end: Int
    /// Its words, kept so the chat still reads if the paragraph changes.
    public var text: String

    /// The passage of `paragraph` from `start` to `end`, in either order.
    /// Nil when either is outside the paragraph or it holds only spaces.
    public init?(in paragraph: Paragraph, from start: Int, to end: Int) {
        let (start, end) = (min(start, end), max(start, end))
        let count = paragraph.text.count
        guard start >= 0, end <= count, start < end else { return nil }
        let from = paragraph.text.index(paragraph.text.startIndex, offsetBy: start)
        let to = paragraph.text.index(paragraph.text.startIndex, offsetBy: end)
        let text = paragraph.text[from..<to].trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return nil }
        self.paragraph = paragraph.id
        self.start = start
        self.end = end
        self.text = text
    }

    /// Whether the character at `offset` of its paragraph is in it.
    public func contains(_ offset: Int) -> Bool { offset >= start && offset < end }
}

/// A message in a chat about what the person is reading.
public struct ReaderMessage: ChatEntry {
    public var id: UUID
    public var role: ChatRole
    public var text: String
    public var at: Date

    public init(id: UUID = UUID(), role: ChatRole, text: String, at: Date) {
        self.id = id
        self.role = role
        self.text = text
        self.at = at
    }
}

/// A conversation about a passage, kept with its book.
public struct AnchoredChat: Identifiable, Hashable, Codable, Sendable {
    public var passage: Passage
    public var conversation: Conversation<ReaderMessage, NoContext>
    /// What it runs on in place of the defaults.
    public var model: ChatModel

    public var id: UUID { conversation.id }

    public init(passage: Passage, createdAt: Date) {
        self.passage = passage
        conversation = Conversation(id: UUID(), createdAt: createdAt)
        model = ChatModel()
    }

    private enum CodingKeys: String, CodingKey { case passage, conversation, model }

    /// A chat kept before chats picked their model follows the defaults.
    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        passage = try container.decode(Passage.self, forKey: .passage)
        conversation = try container.decode(Conversation<ReaderMessage, NoContext>.self, forKey: .conversation)
        model = try container.decodeIfPresent(ChatModel.self, forKey: .model) ?? ChatModel()
    }
}

/// The model a chat picked, how long it thinks and its window, as kept
/// with the book. Each nil follows the default.
public struct ChatModel: Hashable, Codable, Sendable {
    /// The model's ID.
    public var model: String?
    /// How long it thinks: off, on or long.
    public var thinking: String?
    /// Its window in tokens.
    public var window: Int?

    public init(model: String? = nil, thinking: String? = nil, window: Int? = nil) {
        self.model = model
        self.thinking = thinking
        self.window = window
    }
}
