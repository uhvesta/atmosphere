import Foundation

/// Who wrote a message.
public enum ChatRole: String, Hashable, Codable, Sendable {
    case person, assistant
}

/// A message an app keeps in a conversation. The app adds what else it
/// needs, such as attachments or what the assistant asked to change.
public protocol ChatEntry: Identifiable, Hashable, Codable, Sendable where ID == UUID {
    var id: UUID { get }
    var role: ChatRole { get }
    var text: String { get set }
    var at: Date { get }
}

/// What an app keeps with a conversation besides its messages, such as a
/// draft or the changes its replies saved. Read and written with the
/// conversation's own fields, beside them, so adding one keeps what was
/// saved before, so its decoder should read a missing field as empty. A
/// fork starts with an empty one.
public protocol ConversationContext: Codable, Hashable, Sendable {
    init()
}

/// A conversation that keeps nothing besides its messages.
public struct NoContext: ConversationContext {
    public init() {}
}

/// One conversation: its messages as a tree of branches, its summaries,
/// name and where it's kept. The app's `Context` is reached as if it were
/// the conversation's own: `conversation.draft`.
@dynamicMemberLookup
public struct Conversation<Message: ChatEntry, Context: ConversationContext>: Identifiable, Hashable, Sendable {
    public var id: UUID
    /// The messages shown, from the first to where the conversation is now.
    /// Change them with `append` and `go(to:)`, which keep the branches.
    public var messages: [Message] = []
    /// The messages not shown: those after the point the person went back
    /// to, and those of other branches.
    public var hidden: [Message] = []
    /// The message each one follows. One missing follows the one before it
    /// in `messages`, as in conversations kept before branches.
    public var parents: [UUID: UUID] = [:]
    /// The message last shown after each one, which going forward follows.
    public var latest: [UUID: UUID] = [:]
    public var createdAt: Date
    /// When the person archived it; nil while it's in the main list.
    public var archivedAt: Date?
    /// Whether it's kept in the cloud. Conversations start on the device.
    public var isSynced = false
    /// The title the person or the assistant gave it; nil for the first
    /// message.
    public var name: String?
    /// Whether the person gave it its name, which the assistant then never
    /// replaces.
    public var isNamedByHand = false
    /// What the assistant wrote of the conversation, each kept under the
    /// last message it covers. A summary goes with every branch through
    /// that message, as what came before it is the same on each.
    public var summaries: [UUID: ChatSummary] = [:]
    /// What the app keeps with it.
    public var context = Context()

    public init(id: UUID, createdAt: Date) {
        self.id = id
        self.createdAt = createdAt
    }

    public subscript<Value>(dynamicMember keyPath: WritableKeyPath<Context, Value>) -> Value {
        get { context[keyPath: keyPath] }
        set { context[keyPath: keyPath] = newValue }
    }

    /// Its name, or else the person's first message, cut short.
    public var title: String {
        if let name, !name.isEmpty { return name }
        guard let first = messages.first(where: { $0.role == .person && !$0.text.isEmpty })?.text else {
            return "New conversation"
        }
        return first.count > 40 ? String(first.prefix(39)) + "…" : first
    }

    /// When its last message was written, on any branch: going back
    /// doesn't make a conversation older.
    public var updatedAt: Date { (messages + hidden).map(\.at).max() ?? createdAt }

    public var isArchived: Bool { archivedAt != nil }

    /// Names it as the person typed it; an empty name goes back to the
    /// first message and lets the assistant name it again.
    public mutating func rename(_ text: String) {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        name = text.isEmpty ? nil : text
        isNamedByHand = !text.isEmpty
    }
}

extension Conversation: Codable {
    private enum CodingKeys: String, CodingKey {
        case id, messages, hidden, parents, latest, createdAt, archivedAt, isSynced, name, isNamedByHand, summaries
    }

    /// Fields saved before they existed read as their defaults: in one
    /// line, in the main list, on the device and unnamed.
    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UUID.self, forKey: .id)
        messages = try container.decodeIfPresent([Message].self, forKey: .messages) ?? []
        hidden = try container.decodeIfPresent([Message].self, forKey: .hidden) ?? []
        parents = try container.decodeIfPresent([UUID: UUID].self, forKey: .parents) ?? [:]
        latest = try container.decodeIfPresent([UUID: UUID].self, forKey: .latest) ?? [:]
        createdAt = try container.decode(Date.self, forKey: .createdAt)
        archivedAt = try container.decodeIfPresent(Date.self, forKey: .archivedAt)
        isSynced = try container.decodeIfPresent(Bool.self, forKey: .isSynced) ?? false
        name = try container.decodeIfPresent(String.self, forKey: .name)
        isNamedByHand = try container.decodeIfPresent(Bool.self, forKey: .isNamedByHand) ?? false
        summaries = try container.decodeIfPresent([UUID: ChatSummary].self, forKey: .summaries) ?? [:]
        context = try Context(from: decoder)
    }

    public func encode(to encoder: any Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(id, forKey: .id)
        try container.encode(messages, forKey: .messages)
        try container.encode(hidden, forKey: .hidden)
        try container.encode(parents, forKey: .parents)
        try container.encode(latest, forKey: .latest)
        try container.encode(createdAt, forKey: .createdAt)
        try container.encodeIfPresent(archivedAt, forKey: .archivedAt)
        try container.encode(isSynced, forKey: .isSynced)
        try container.encodeIfPresent(name, forKey: .name)
        try container.encode(isNamedByHand, forKey: .isNamedByHand)
        try container.encode(summaries, forKey: .summaries)
        try context.encode(to: encoder)
    }
}
