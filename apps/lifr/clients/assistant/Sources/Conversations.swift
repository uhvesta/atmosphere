import ComposableArchitecture
import Foundation
import LifrCore

/// One conversation with the assistant.
package struct Conversation: Identifiable, Hashable, Codable, Sendable {
    package var id: UUID
    package var messages: [ChatMessage] = []
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

    package init(id: UUID, createdAt: Date) {
        self.id = id
        self.createdAt = createdAt
    }

    /// The person's first message, cut short.
    package var title: String {
        guard let first = messages.first(where: { $0.role == .person && !$0.text.isEmpty })?.text else {
            return "New conversation"
        }
        return first.count > 40 ? String(first.prefix(39)) + "…" : first
    }

    package var updatedAt: Date { messages.last?.at ?? createdAt }

    package var isArchived: Bool { archivedAt != nil }
}

extension Conversation {
    private enum CodingKeys: String, CodingKey {
        case id, messages, draft, changes, added, createdAt, archivedAt, isSynced
    }

    /// Conversations saved before archiving and per-conversation syncing
    /// read as in the main list and on this device.
    package init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UUID.self, forKey: .id)
        messages = try container.decodeIfPresent([ChatMessage].self, forKey: .messages) ?? []
        draft = try container.decodeIfPresent(Plan.self, forKey: .draft)
        changes = try container.decodeIfPresent([UUID: [UUID]].self, forKey: .changes) ?? [:]
        added = try container.decodeIfPresent([UUID].self, forKey: .added) ?? []
        createdAt = try container.decode(Date.self, forKey: .createdAt)
        archivedAt = try container.decodeIfPresent(Date.self, forKey: .archivedAt)
        isSynced = try container.decodeIfPresent(Bool.self, forKey: .isSynced) ?? false
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
