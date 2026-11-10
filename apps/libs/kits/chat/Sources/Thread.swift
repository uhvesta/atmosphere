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
    /// The model that wrote a reply, by name; nil for the person's
    /// messages and replies kept before it was.
    public var model: String?
    /// How fast the model read the chat and wrote a reply; nil when it
    /// wasn't measured.
    public var speed: ThreadSpeed?
    /// Whether the person stopped the reply before the model finished it.
    public var stopped: Bool

    public init(
        id: UUID = UUID(), role: ChatRole, text: String, at: Date, files: [ThreadFile] = [],
        photos: [ThreadPhoto] = [], thinking: ThreadThinking? = nil, model: String? = nil, speed: ThreadSpeed? = nil,
        stopped: Bool = false
    ) {
        self.id = id
        self.role = role
        self.text = text
        self.at = at
        self.files = files
        self.photos = photos
        self.thinking = thinking
        self.model = model
        self.speed = speed
        self.stopped = stopped
    }

    private enum CodingKeys: String, CodingKey {
        case id, role, text, at, files, photos, thinking, model, speed, stopped
    }

    /// A message kept before files, photos, thinking, its model or its
    /// speed were kept has none, and one kept before stopping was kept
    /// wasn't stopped.
    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UUID.self, forKey: .id)
        role = try container.decode(ChatRole.self, forKey: .role)
        text = try container.decode(String.self, forKey: .text)
        at = try container.decode(Date.self, forKey: .at)
        files = try container.decodeIfPresent([ThreadFile].self, forKey: .files) ?? []
        photos = try container.decodeIfPresent([ThreadPhoto].self, forKey: .photos) ?? []
        thinking = try container.decodeIfPresent(ThreadThinking.self, forKey: .thinking)
        model = try container.decodeIfPresent(String.self, forKey: .model)
        speed = try container.decodeIfPresent(ThreadSpeed.self, forKey: .speed)
        stopped = try container.decodeIfPresent(Bool.self, forKey: .stopped) ?? false
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

/// How fast a model read a chat and wrote a reply to it.
public struct ThreadSpeed: Hashable, Codable, Sendable {
    /// The tokens it read and wrote.
    public var read: Int
    public var written: Int
    /// How long it read for, until its first token, and how long it wrote
    /// for after that.
    public var reading: Duration
    public var writing: Duration
    /// Whether the counts are guessed from the text's length, for a model
    /// that doesn't count its tokens.
    public var estimated: Bool

    public init(read: Int, written: Int, reading: Duration, writing: Duration, estimated: Bool = false) {
        self.read = read
        self.written = written
        self.reading = reading
        self.writing = writing
        self.estimated = estimated
    }

    /// Tokens read a second, or nil when it took no measurable time.
    public var readPerSecond: Double? { Self.rate(read, in: reading) }
    /// Tokens written a second, or nil when it took no measurable time.
    public var writtenPerSecond: Double? { Self.rate(written, in: writing) }

    private static func rate(_ tokens: Int, in duration: Duration) -> Double? {
        let seconds = Double(duration.components.seconds) + Double(duration.components.attoseconds) / 1e18
        return seconds > 0.001 ? Double(tokens) / seconds : nil
    }
}

/// A conversation of plain messages.
public typealias ChatThread = Conversation<ThreadMessage, NoContext>
