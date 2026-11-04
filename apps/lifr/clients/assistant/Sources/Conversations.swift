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
    /// The skills attached as pills, in the order they were attached.
    package var skills: [AttachedSkill] = []
    package var createdAt: Date

    package init(id: UUID, createdAt: Date) {
        self.id = id
        self.createdAt = createdAt
    }

    /// The person's first message, cut short.
    package var title: String {
        guard let first = messages.first(where: { $0.role == .person })?.text else { return "New conversation" }
        return first.count > 40 ? String(first.prefix(39)) + "…" : first
    }

    package var updatedAt: Date { messages.last?.at ?? createdAt }

    /// The attached skills that still exist and are on, with their notes.
    package func uses(of skills: [Skill]) -> [SkillUse] {
        let byID = Dictionary(skills.map { ($0.id, $0) }) { $1 }
        return self.skills.compactMap { attached in
            byID[attached.id].flatMap { $0.isOn ? SkillUse($0, note: attached.note) : nil }
        }
    }
}

extension Conversation {
    private enum CodingKeys: String, CodingKey { case id, messages, draft, changes, added, skills, createdAt }

    /// Conversations saved before skills existed have none.
    package init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UUID.self, forKey: .id)
        messages = try container.decode([ChatMessage].self, forKey: .messages)
        draft = try container.decodeIfPresent(Plan.self, forKey: .draft)
        changes = try container.decode([UUID: [UUID]].self, forKey: .changes)
        added = try container.decode([UUID].self, forKey: .added)
        skills = try container.decodeIfPresent([AttachedSkill].self, forKey: .skills) ?? []
        createdAt = try container.decode(Date.self, forKey: .createdAt)
    }
}

/// Keeps conversations on this device, or in iCloud when the person turns
/// syncing on. Deleting a conversation never deletes the tasks it changed.
@DependencyClient
package struct ConversationsClient: Sendable {
    /// Every kept conversation, in no particular order.
    package var load: @Sendable () async throws -> [Conversation]
    /// Adds the conversation, or replaces the one with its ID.
    package var save: @Sendable (_ conversation: Conversation) async throws -> Void
    /// Removes conversations here and, if they were synced, from iCloud.
    package var delete: @Sendable (_ ids: [UUID]) async throws -> Void
    /// Removes every conversation here and from iCloud.
    package var deleteAll: @Sendable () async throws -> Void
    /// Whether conversations sync through iCloud.
    package var isSyncing: @Sendable () -> Bool = { false }
    /// Turns syncing on, moving this device's conversations to iCloud, or
    /// off, keeping a copy of them here. Copies already in iCloud stay
    /// there until deleted.
    package var setSyncing: @Sendable (_ on: Bool) async throws -> Void
}

extension ConversationsClient {
    /// Keeps conversations in memory, for previews and tests.
    package static func inMemory(_ conversations: [Conversation] = []) -> Self {
        let stored = LockIsolated(Dictionary(uniqueKeysWithValues: conversations.map { ($0.id, $0) }))
        let syncing = LockIsolated(false)
        return ConversationsClient(
            load: { Array(stored.value.values) },
            save: { conversation in stored.withValue { $0[conversation.id] = conversation } },
            delete: { ids in stored.withValue { for id in ids { $0[id] = nil } } },
            deleteAll: { stored.setValue([:]) },
            isSyncing: { syncing.value },
            setSyncing: { syncing.setValue($0) }
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
