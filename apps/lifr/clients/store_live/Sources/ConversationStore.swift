import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrBlob
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

/// Conversations and skills on SwiftData. Each conversation is kept in a
/// local-only store, or in the synced store once the person syncs it.
/// Skills are always in the synced store. The synced store is its own,
/// mirrored by SwiftData; the records sync another way, through a merge.
package actor ConversationStore {
    private let synced: ModelContainer
    private let local: ModelContainer

    /// `synced` syncs through iCloud; `local` never does.
    package init(synced: ModelContainer, local: ModelContainer) {
        self.synced = synced
        self.local = local
    }

    /// The stores on disk, with what an earlier Lifr kept in `records`,
    /// the Lifr container, moved into the synced one.
    package static func onDisk(movingFrom records: ModelContainer) throws(ConversationStoreError) -> ConversationStore {
        let synced = try container(ModelConfiguration("Shared", schema: schema, cloudKitDatabase: .automatic))
        try move(from: records, to: synced)
        return ConversationStore(synced: synced, local: try localOnDisk())
    }

    /// Moves every conversation and skill from `source` into `target`; a
    /// newer copy already in `target` wins.
    package static func move(from source: ModelContainer, to target: ModelContainer) throws(ConversationStoreError) {
        do {
            let from = ModelContext(source)
            let conversations = try from.fetch(FetchDescriptor<StoredConversation>())
            let skills = try from.fetch(FetchDescriptor<StoredSkill>())
            guard !conversations.isEmpty || !skills.isEmpty else { return }
            let to = ModelContext(target)
            for stored in conversations {
                let id = stored.id
                let existing = try to.fetch(FetchDescriptor<StoredConversation>(predicate: #Predicate { $0.id == id }))
                if let newest = existing.map(\.updatedAt).max(), newest > stored.updatedAt { continue }
                existing.forEach(to.delete)
                let record = StoredConversation(id: id)
                record.updatedAt = stored.updatedAt
                record.conversation = stored.conversation
                to.insert(record)
            }
            for skill in skills {
                try upsert(id: skill.id, updatedAt: skill.updatedAt, data: skill.skill, in: to)
            }
            try to.save()
            try from.delete(model: StoredConversation.self)
            try from.delete(model: StoredSkill.self)
            try from.save()
        } catch {
            throw .openFailed(error)
        }
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

    /// Both stores' conversations, each marked with the one it's in. If a
    /// move was cut short and both have it, the newer copy wins.
    package func load() throws(ConversationStoreError) -> [Conversation] {
        var byID: [UUID: Conversation] = [:]
        for (container, isSynced) in [(local, false), (synced, true)] {
            for var conversation in try Self.conversations(in: container) {
                conversation.isSynced = isSynced
                if let kept = byID[conversation.id], kept.updatedAt > conversation.updatedAt { continue }
                byID[conversation.id] = conversation
            }
        }
        return Array(byID.values)
    }

    private static func conversations(in container: ModelContainer) throws(ConversationStoreError) -> [Conversation] {
        let stored: [StoredConversation]
        do {
            stored = try ModelContext(container).fetch(FetchDescriptor<StoredConversation>())
        } catch {
            throw .readFailed(error)
        }
        // A conversation that doesn't decode is skipped rather than hiding
        // the rest.
        return stored.compactMap { record in
            do {
                return try Blob.decode(Conversation.self, from: record.conversation ?? Data())
            } catch {
                Logger(subsystem: "lifr", category: "conversations")
                    .error("Skipped conversation \(record.id, privacy: .public): \(String(describing: error))")
                return nil
            }
        }
    }

    /// Saves into the store `isSynced` names, then removes any copy from
    /// the other one.
    package func save(_ conversation: Conversation) throws(ConversationStoreError) {
        let (target, other) = conversation.isSynced ? (synced, local) : (local, synced)
        do {
            let context = ModelContext(target)
            try Self.upsert(conversation, in: context)
            try context.save()
            let id = conversation.id
            let leaving = ModelContext(other)
            try leaving.delete(model: StoredConversation.self, where: #Predicate { $0.id == id })
            try leaving.save()
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

    // MARK: - Skills

    /// The built-in skills, then the person's. A stored skill that doesn't
    /// decode is skipped.
    package func loadSkills() throws(ConversationStoreError) -> [Skill] {
        try moveLocalSkills()
        let stored: [StoredSkill]
        do {
            stored = try ModelContext(synced).fetch(FetchDescriptor<StoredSkill>())
        } catch {
            throw .skillsReadFailed(error)
        }
        let skills = stored.compactMap { record -> Skill? in
            do {
                return try Blob.decode(Skill.self, from: record.skill ?? Data())
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
            let context = ModelContext(synced)
            try Self.upsert(id: skill.id, updatedAt: now, data: Blob.encode(skill), in: context)
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

    /// Moves skills kept on this device, from when skills followed a switch
    /// for all conversations, into the synced store.
    private func moveLocalSkills() throws(ConversationStoreError) {
        do {
            let source = ModelContext(local)
            let skills = try source.fetch(FetchDescriptor<StoredSkill>())
            guard !skills.isEmpty else { return }
            let target = ModelContext(synced)
            for skill in skills {
                try Self.upsert(id: skill.id, updatedAt: skill.updatedAt, data: skill.skill, in: target)
            }
            try target.save()
            try source.delete(model: StoredSkill.self)
            try source.save()
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

    /// Adds or replaces the conversation; a newer copy already there, such
    /// as one from another device, wins.
    private static func upsert(_ conversation: Conversation, in context: ModelContext) throws {
        let id = conversation.id
        let existing = try context.fetch(FetchDescriptor<StoredConversation>(predicate: #Predicate { $0.id == id }))
        if let newest = existing.map(\.updatedAt).max(), newest > conversation.updatedAt { return }
        existing.forEach(context.delete)
        let record = StoredConversation(id: id)
        record.updatedAt = conversation.updatedAt
        record.conversation = try Blob.encode(conversation)
        context.insert(record)
    }
}

extension ConversationsClient {
    package static func live(_ store: ConversationStore) -> Self {
        ConversationsClient(
            load: { try await store.load() },
            save: { try await store.save($0) },
            delete: { try await store.delete($0) }
        )
    }
}

extension ConversationsClient {
    /// Fails every call with `error`, for when the stores didn't open.
    package static func failing(_ error: ConversationStoreError) -> Self {
        ConversationsClient(
            load: { throw error }, save: { _ in throw error }, delete: { _ in throw error })
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
