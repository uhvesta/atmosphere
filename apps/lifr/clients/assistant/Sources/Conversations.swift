import ComposableArchitecture
import Foundation
import LifrCore

/// One conversation with the assistant.
package struct Conversation: Identifiable, Hashable, Codable, Sendable {
    package var id: UUID
    /// The messages shown, from the first to where the conversation is now.
    /// Change them with `append` and `go(to:)`, which keep the branches.
    package var messages: [ChatMessage] = []
    /// The messages not shown: those after the point the person went back
    /// to, and those of other branches.
    package var hidden: [ChatMessage] = []
    /// The message each one follows. One missing follows the one before it
    /// in `messages`, as in conversations kept before branches.
    package var parents: [UUID: UUID] = [:]
    /// The message last shown after each one, which going forward follows.
    package var latest: [UUID: UUID] = [:]
    /// The plan on the draft card.
    package var draft: Plan?
    /// The journal entries each assistant message saved, for its Undo.
    package var changes: [UUID: [UUID]] = [:]
    /// The journal entries of plans added from the draft card.
    package var added: [UUID] = []
    package var createdAt: Date
    /// When the person archived it; nil while it's in the main list.
    package var archivedAt: Date?
    /// Whether it's kept in iCloud. Conversations start on this device.
    package var isSynced = false
    /// The title the person or the assistant gave it; nil for the first
    /// message.
    package var name: String?
    /// Whether the person gave it its name, which the assistant then never
    /// replaces.
    package var isNamedByHand = false
    /// What the assistant wrote of the conversation, each kept under the
    /// last message it covers. A summary goes with every branch through
    /// that message, as what came before it is the same on each.
    package var summaries: [UUID: ChatSummary] = [:]

    package init(id: UUID, createdAt: Date) {
        self.id = id
        self.createdAt = createdAt
    }

    /// Its name, or else the person's first message, cut short.
    package var title: String {
        if let name, !name.isEmpty { return name }
        guard let first = messages.first(where: { $0.role == .person && !$0.text.isEmpty })?.text else {
            return "New conversation"
        }
        return first.count > 40 ? String(first.prefix(39)) + "…" : first
    }

    /// When its last message was written, on any branch: going back
    /// doesn't make a conversation older.
    package var updatedAt: Date { (messages + hidden).map(\.at).max() ?? createdAt }

    package var isArchived: Bool { archivedAt != nil }
}

extension Conversation {
    private enum CodingKeys: String, CodingKey {
        case id, messages, hidden, parents, latest, draft, changes, added, createdAt, archivedAt, isSynced, name
        case isNamedByHand, summaries
    }

    /// Conversations saved before archiving, per-conversation syncing,
    /// names and branches read as in the main list, on this device, unnamed
    /// and in one line.
    package init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UUID.self, forKey: .id)
        messages = try container.decodeIfPresent([ChatMessage].self, forKey: .messages) ?? []
        hidden = try container.decodeIfPresent([ChatMessage].self, forKey: .hidden) ?? []
        parents = try container.decodeIfPresent([UUID: UUID].self, forKey: .parents) ?? [:]
        latest = try container.decodeIfPresent([UUID: UUID].self, forKey: .latest) ?? [:]
        draft = try container.decodeIfPresent(Plan.self, forKey: .draft)
        changes = try container.decodeIfPresent([UUID: [UUID]].self, forKey: .changes) ?? [:]
        added = try container.decodeIfPresent([UUID].self, forKey: .added) ?? []
        createdAt = try container.decode(Date.self, forKey: .createdAt)
        archivedAt = try container.decodeIfPresent(Date.self, forKey: .archivedAt)
        isSynced = try container.decodeIfPresent(Bool.self, forKey: .isSynced) ?? false
        name = try container.decodeIfPresent(String.self, forKey: .name)
        isNamedByHand = try container.decodeIfPresent(Bool.self, forKey: .isNamedByHand) ?? false
        summaries = try container.decodeIfPresent([UUID: ChatSummary].self, forKey: .summaries) ?? [:]
    }

    /// The newest summary of the messages shown, and the message it ends
    /// at: nil when none of them is summarized.
    package var summary: (through: UUID, summary: ChatSummary)? {
        for message in messages.reversed() {
            if let summary = summaries[message.id] { return (message.id, summary) }
        }
        return nil
    }

    /// The messages shown after the newest summary: all of them without one.
    package var unsummarized: ArraySlice<ChatMessage> {
        guard let through = summary?.through, let index = messages.firstIndex(where: { $0.id == through }) else {
            return messages[...]
        }
        return messages[(index + 1)...]
    }

    /// The messages a new summary would cover: those after the newest one
    /// but the last two, which stay as they were written so the model
    /// sees the latest exchange word for word.
    package var compactable: ArraySlice<ChatMessage> { unsummarized.dropLast(Self.keptWhole) }

    /// The messages a summary leaves as they are.
    package static let keptWhole = 2

    /// Names it as the person typed it; an empty name goes back to the
    /// first message and lets the assistant name it again.
    package mutating func rename(_ text: String) {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        name = text.isEmpty ? nil : text
        isNamedByHand = !text.isEmpty
    }
}

/// What the assistant wrote of a conversation up to one message, sent in
/// place of those messages so a long conversation fits the model.
package struct ChatSummary: Hashable, Codable, Sendable {
    package var text: String
    package var at: Date

    package init(text: String, at: Date) {
        self.text = text
        self.at = at
    }
}

extension SharedKey where Self == AppStorageKey<Bool>.Default {
    /// Whether the assistant summarizes older messages by itself once they
    /// no longer fit what the model reads. Off unless the person turns it
    /// on: otherwise the chat offers Compact.
    package static var compactsConversations: Self {
        Self[.appStorage("lifrCompactsConversations"), default: false]
    }

    /// Whether the assistant names each new conversation after its first
    /// reply. Off unless the person turns it on.
    package static var namesConversations: Self {
        Self[.appStorage("lifrNamesConversations"), default: false]
    }
}

/// Keeps each conversation on this device, or in iCloud when it's synced.
/// Deleting a conversation never deletes the tasks it changed.
@DependencyClient
package struct ConversationsClient: Sendable {
    /// Every kept conversation, in no particular order.
    package var load: @Sendable () async throws -> [Conversation]
    /// Adds the conversation, or replaces the one with its ID, where its
    /// `isSynced` says: a synced one moves to iCloud, and one that stops
    /// syncing leaves iCloud and stays here.
    package var save: @Sendable (_ conversation: Conversation) async throws -> Void
    /// Removes conversations here and, if they were synced, from iCloud.
    package var delete: @Sendable (_ ids: [UUID]) async throws -> Void
}

extension ConversationsClient {
    /// Keeps conversations in memory, for previews and tests.
    package static func inMemory(_ conversations: [Conversation] = []) -> Self {
        let stored = LockIsolated(Dictionary(uniqueKeysWithValues: conversations.map { ($0.id, $0) }))
        return ConversationsClient(
            load: { Array(stored.value.values) },
            save: { conversation in stored.withValue { $0[conversation.id] = conversation } },
            delete: { ids in stored.withValue { for id in ids { $0[id] = nil } } }
        )
    }
}

extension ConversationsClient: TestDependencyKey {
    package static let testValue = ConversationsClient()
    package static let previewValue = ConversationsClient.inMemory()
}

extension DependencyValues {
    package var conversations: ConversationsClient {
        get { self[ConversationsClient.self] }
        set { self[ConversationsClient.self] = newValue }
    }
}
