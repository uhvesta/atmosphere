import Foundation

/// A message in a plain chat: text, the photos and files sent with it, and
/// what the model thought before replying. For apps whose chats keep
/// nothing else.
public struct ThreadMessage: ChatEntry {
    public var id: UUID
    public var role: ChatRole
    public var text: String
    public var at: Date
    /// The files sent with it, whose text the model read.
    public var files: [ThreadFile]
    /// The photos sent with it, whose files the app keeps beside the chat.
    public var photos: [ThreadPhoto]
    /// What the model thought before writing it; nil for the person's
    /// messages and replies written straight away.
    public var thinking: ThreadThinking?

    public init(
        id: UUID = UUID(), role: ChatRole, text: String, at: Date, files: [ThreadFile] = [],
        photos: [ThreadPhoto] = [], thinking: ThreadThinking? = nil
    ) {
        self.id = id
        self.role = role
        self.text = text
        self.at = at
        self.files = files
        self.photos = photos
        self.thinking = thinking
    }

    private enum CodingKeys: String, CodingKey { case id, role, text, at, files, photos, thinking }

    /// A message kept before files, photos or thinking were kept has none.
    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UUID.self, forKey: .id)
        role = try container.decode(ChatRole.self, forKey: .role)
        text = try container.decode(String.self, forKey: .text)
        at = try container.decode(Date.self, forKey: .at)
        files = try container.decodeIfPresent([ThreadFile].self, forKey: .files) ?? []
        photos = try container.decodeIfPresent([ThreadPhoto].self, forKey: .photos) ?? []
        thinking = try container.decodeIfPresent(ThreadThinking.self, forKey: .thinking)
    }
}

/// A file sent with a message: its name and the text read from it.
public struct ThreadFile: Hashable, Codable, Sendable {
    public var name: String
    public var text: String

    public init(name: String, text: String) {
        self.name = name
        self.text = text
    }
}

/// A photo sent with a message. Only its ID is in the chat: the app keeps
/// the photo itself, on the device.
public struct ThreadPhoto: Identifiable, Hashable, Codable, Sendable {
    public var id: UUID

    public init(id: UUID) {
        self.id = id
    }
}

/// What a model thought before a reply, and how long it took.
public struct ThreadThinking: Hashable, Codable, Sendable {
    public var text: String
    public var duration: Duration?

    public init(text: String, duration: Duration? = nil) {
        self.text = text
        self.duration = duration
    }
}

/// A conversation of plain messages.
public typealias ChatThread = Conversation<ThreadMessage, NoContext>
