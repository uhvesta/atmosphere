import ComposableArchitecture
import Foundation
import LifrAssistantClient
import OSLog
import SwiftData

/// Why conversations couldn't be read or kept.
package enum ConversationStoreError: Error, CustomStringConvertible {
    case openFailed(any Error)
    case readFailed(any Error)
    case saveFailed(any Error)
    case skillsReadFailed(any Error)
    case skillSaveFailed(any Error)

    package var description: String {
        switch self {
        case .openFailed(let error):
            "Couldn't open the conversations on this device (\(error.localizedDescription)). "
                + "Quit and reopen Lifr; if it keeps happening, free up some storage."
        case .readFailed(let error):
            "Couldn't read the conversations (\(error.localizedDescription)). Your tasks aren't affected. "
                + "Reopen Lifr and try again."
        case .saveFailed(let error):
            "Couldn't save the conversation (\(error.localizedDescription)). Nothing was changed. "
                + "Try again; if storage is full, free some up first."
        case .skillsReadFailed(let error):
            "Couldn't read your skills (\(error.localizedDescription)). The built-in skills still work. "
                + "Reopen Lifr and try again."
        case .skillSaveFailed(let error):
            "Couldn't save the skill (\(error.localizedDescription)). Nothing was changed. "
                + "Try again; if storage is full, free some up first."
        }
    }
}

/// Conversations and skills on SwiftData: in a local-only store, or in the
/// synced Lifr store when the person turns syncing on. The choice is kept
/// per device.
package actor ConversationStore {
    private let synced: ModelContainer
    private let local: ModelContainer
    private let defaults: UserDefaults
    private static let syncingKey = "lifr.syncConversations"

    /// `synced` is the Lifr container; `local` never syncs.
    package init(synced: ModelContainer, local: ModelContainer, defaults: UserDefaults = .standard) {
        self.synced = synced
        self.local = local
        self.defaults = defaults
    }

    /// The local container, on disk as Conversations.store next to Lifr.store.
    package static func localOnDisk() throws(ConversationStoreError) -> ModelContainer {
        try container(ModelConfiguration("Conversations", schema: schema, cloudKitDatabase: .none))
    }

    /// A local container that is never written to disk, for tests.
    package static func localInMemory() throws(ConversationStoreError) -> ModelContainer {
        try container(ModelConfiguration(schema: schema, isStoredInMemoryOnly: true, cloudKitDatabase: .none))
    }

    private static let schema = Schema([StoredConversation.self, StoredSkill.self])

    private static func container(_ configuration: ModelConfiguration) throws(ConversationStoreError) -> ModelContainer
    {
        do {
            return try ModelContainer(for: schema, configurations: configuration)
        } catch {
            throw .openFailed(error)
        }
    }

    package nonisolated var isSyncing: Bool { defaults.bool(forKey: Self.syncingKey) }

    private var current: ModelContainer { isSyncing ? synced : local }

    package func load() throws(ConversationStoreError) -> [Conversation] {
        let stored: [StoredConversation]
        do {
            stored = try ModelContext(current).fetch(FetchDescriptor<StoredConversation>())
        } catch {
            throw .readFailed(error)
        }
        // A conversation that doesn't decode is skipped rather than hiding
        // the rest.
        return stored.compactMap { record in
            do {
                return try JSONDecoder().decode(Conversation.self, from: record.conversation ?? Data())
            } catch {
                Logger(subsystem: "lifr", category: "conversations")
                    .error("Skipped conversation \(record.id, privacy: .public): \(String(describing: error))")
                return nil
            }
        }
    }

    package func save(_ conversation: Conversation) throws(ConversationStoreError) {
        do {
            let context = ModelContext(current)
            try Self.upsert([conversation], in: context)
            try context.save()
        } catch {
            throw .saveFailed(error)
        }
    }

    /// Deletes from both stores, so a conversation goes from iCloud too.
    package func delete(_ ids: [UUID]) throws(ConversationStoreError) {
        do {
            for container in [local, synced] {
                let context = ModelContext(container)
                try context.delete(model: StoredConversation.self, where: #Predicate { ids.contains($0.id) })
                try context.save()
            }
        } catch {
            throw .saveFailed(error)
        }
    }

    package func deleteAll() throws(ConversationStoreError) {
        do {
            for container in [local, synced] {
                let context = ModelContext(container)
                try context.delete(model: StoredConversation.self)
                try context.save()
            }
        } catch {
            throw .saveFailed(error)
        }
    }

    /// On: moves local conversations and skills into the synced store. Off:
    /// copies the synced ones here and leaves them in iCloud.
    package func setSyncing(_ on: Bool) throws(ConversationStoreError) {
        guard on != isSyncing else { return }
        let conversations = try load()
        let skills: [StoredSkill]
        do {
            skills = try ModelContext(current).fetch(FetchDescriptor<StoredSkill>())
        } catch {
            throw .skillsReadFailed(error)
        }
        do {
            let target = ModelContext(on ? synced : local)
            try Self.upsert(conversations, in: target)
            for skill in skills {
                try Self.upsert(id: skill.id, updatedAt: skill.updatedAt, data: skill.skill, in: target)
            }
            try target.save()
            if on {
                let source = ModelContext(local)
                try source.delete(model: StoredConversation.self)
                try source.delete(model: StoredSkill.self)
                try source.save()
            }
        } catch {
            throw .saveFailed(error)
        }
        defaults.set(on, forKey: Self.syncingKey)
    }

    // MARK: - Skills

    /// The built-in skills, then the person's. A stored skill that doesn't
    /// decode is skipped.
    package func loadSkills() throws(ConversationStoreError) -> [Skill] {
        let stored: [StoredSkill]
        do {
            stored = try ModelContext(current).fetch(FetchDescriptor<StoredSkill>())
        } catch {
            throw .skillsReadFailed(error)
        }
        let skills = stored.compactMap { record -> Skill? in
            do {
                return try JSONDecoder().decode(Skill.self, from: record.skill ?? Data())
            } catch {
                Logger(subsystem: "lifr", category: "conversations")
                    .error("Skipped skill \(record.id, privacy: .public): \(String(describing: error))")
                return nil
            }
        }
        return Skill.merged(stored: skills)
    }

    /// Adds or replaces the skill, as of `now`.
    package func saveSkill(_ skill: Skill, now: Date = .now) throws(ConversationStoreError) {
        do {
            let context = ModelContext(current)
            try Self.upsert(id: skill.id, updatedAt: now, data: JSONEncoder().encode(skill), in: context)
            try context.save()
        } catch {
            throw .skillSaveFailed(error)
        }
    }

    /// Deletes from both stores, so a skill goes from iCloud too.
    package func deleteSkills(_ ids: [String]) throws(ConversationStoreError) {
        do {
            for container in [local, synced] {
                let context = ModelContext(container)
                try context.delete(model: StoredSkill.self, where: #Predicate { ids.contains($0.id) })
                try context.save()
            }
        } catch {
            throw .skillSaveFailed(error)
        }
    }

    /// Adds or replaces a stored skill; a newer copy already there wins.
    private static func upsert(id: String, updatedAt: Date, data: Data?, in context: ModelContext) throws {
        let existing = try context.fetch(FetchDescriptor<StoredSkill>(predicate: #Predicate { $0.id == id }))
        if let newest = existing.map(\.updatedAt).max(), newest > updatedAt { return }
        existing.forEach(context.delete)
        let record = StoredSkill(id: id)
        record.updatedAt = updatedAt
        record.skill = data
        context.insert(record)
    }

    /// Adds or replaces each conversation; a newer copy already there wins.
    private static func upsert(_ conversations: [Conversation], in context: ModelContext) throws {
        let encoder = JSONEncoder()
        for conversation in conversations {
            let id = conversation.id
            let existing = try context.fetch(FetchDescriptor<StoredConversation>(predicate: #Predicate { $0.id == id }))
            if let newest = existing.map(\.updatedAt).max(), newest > conversation.updatedAt { continue }
            existing.forEach(context.delete)
            let record = StoredConversation(id: id)
            record.updatedAt = conversation.updatedAt
            record.conversation = try encoder.encode(conversation)
            context.insert(record)
        }
    }
}

extension ConversationsClient {
    package static func live(_ store: ConversationStore) -> Self {
        ConversationsClient(
            load: { try await store.load() },
            save: { try await store.save($0) },
            delete: { try await store.delete($0) },
            deleteAll: { try await store.deleteAll() },
            isSyncing: { store.isSyncing },
            setSyncing: { try await store.setSyncing($0) }
        )
    }
}

extension ConversationsClient {
    /// Fails every call with `error`, for when the stores didn't open.
    package static func failing(_ error: ConversationStoreError) -> Self {
        ConversationsClient(
            load: { throw error }, save: { _ in throw error }, delete: { _ in throw error },
            deleteAll: { throw error }, isSyncing: { false }, setSyncing: { _ in throw error })
    }
}

extension SkillsClient {
    package static func live(_ store: ConversationStore) -> Self {
        SkillsClient(
            load: { try await store.loadSkills() },
            save: { try await store.saveSkill($0) },
            delete: { try await store.deleteSkills($0) })
    }

    /// Fails every call with `error`, for when the stores didn't open.
    package static func failing(_ error: ConversationStoreError) -> Self {
        SkillsClient(load: { throw error }, save: { _ in throw error }, delete: { _ in throw error })
    }
}
