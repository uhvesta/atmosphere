import Foundation
import LifrAssistantClient
import LifrBlob
import LifrCore
import SwiftData
import XCTest

@testable import LifrStoreLive

final class ConversationStoreTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    /// A store over two in-memory containers, and the synced one to look in.
    private func store() throws -> (ConversationStore, synced: ModelContainerProbe, local: ModelContainerProbe) {
        let synced = try LiveStore.inMemory()
        let local = try ConversationStore.localInMemory()
        return (
            ConversationStore(synced: synced, local: local),
            ModelContainerProbe(synced), ModelContainerProbe(local)
        )
    }

    private func conversation(_ text: String, plan: Plan? = nil) -> Conversation {
        var conversation = Conversation(id: UUID(), createdAt: now)
        conversation.messages = [ChatMessage(id: UUID(), role: .person, text: text, at: now)]
        conversation.draft = plan
        conversation.changes = [UUID(): [UUID()]]
        return conversation
    }

    func testKeepsAConversationOnThisDeviceUntilItsSynced() async throws {
        let (store, synced, local) = try store()
        let chat = conversation("Plan the week", plan: Plan(tasks: [.init(title: "Shop", place: "HOME")]))

        try await store.save(chat)

        let loaded = try await store.load()
        XCTAssertEqual(loaded, [chat])
        XCTAssertEqual(try synced.count(), 0)
        XCTAssertEqual(try local.count(), 1)
    }

    func testAConversationSavedAsPlainJSONStillLoadsAndIsRewrittenAsABlob() async throws {
        let (store, _, local) = try store()
        let chat = conversation("Plan the week", plan: Plan(tasks: [.init(title: "Shop", place: "HOME")]))
        let context = ModelContext(local.container)
        let old = StoredConversation(id: chat.id)
        old.updatedAt = chat.updatedAt
        old.conversation = try JSONEncoder().encode(chat)
        context.insert(old)
        try context.save()

        let loaded = try await store.load()
        XCTAssertEqual(loaded, [chat])

        try await store.save(chat)
        let stored = try XCTUnwrap(try local.conversations().first?.conversation)
        XCTAssertNotEqual(try Blob.format(of: stored), .legacy)
        let reloaded = try await store.load()
        XCTAssertEqual(reloaded, [chat])
    }

    func testSyncingMovesAConversationToICloudAndStoppingBringsItBack() async throws {
        let (store, synced, local) = try store()
        var chat = conversation("Hello")
        let other = conversation("Stays here")
        try await store.save(chat)
        try await store.save(other)

        chat.isSynced = true
        try await store.save(chat)
        XCTAssertEqual(try synced.count(), 1)
        XCTAssertEqual(try local.count(), 1)
        let afterOn = try await store.load()
        XCTAssertEqual(Set(afterOn), [chat, other])

        chat.isSynced = false
        try await store.save(chat)
        XCTAssertEqual(try synced.count(), 0)
        XCTAssertEqual(try local.count(), 2)
        let afterOff = try await store.load()
        XCTAssertEqual(Set(afterOff), [chat, other])
    }

    func testAnArchivedConversationLoadsArchived() async throws {
        let (store, _, _) = try store()
        var chat = conversation("Old")
        chat.archivedAt = now
        try await store.save(chat)

        let loaded = try await store.load()
        XCTAssertEqual(loaded.first?.archivedAt, now)
    }

    func testAConversationSavedBeforeArchivingIsInTheMainListAndLocal() throws {
        let old = Data(#"{"id":"\#(UUID())","messages":[],"changes":[],"added":[],"createdAt":0}"#.utf8)
        let decoded = try JSONDecoder().decode(Conversation.self, from: old)
        XCTAssertNil(decoded.archivedAt)
        XCTAssertFalse(decoded.isSynced)
    }

    func testDeletingRemovesTheConversationFromBothStores() async throws {
        let (store, synced, local) = try store()
        let kept = conversation("Keep")
        var gone = conversation("Gone")
        gone.isSynced = true
        try await store.save(gone)
        try await store.save(kept)

        try await store.delete([gone.id])
        let loaded = try await store.load()
        XCTAssertEqual(loaded, [kept])
        XCTAssertEqual(try synced.count(), 0)
        XCTAssertEqual(try local.count(), 1)
    }

    private func skill(_ name: String) throws -> Skill {
        try XCTUnwrap(
            Skill(userNamed: name, summary: "", instructions: "Be brief.", tools: [.findTasks], draftsPlans: false))
    }

    func testKeepsSkillsAndBuiltInSwitchesWithConversations() async throws {
        let (store, _, _) = try store()
        let mine = try skill("Weekly")
        var undo = Skill.undo
        undo.isOn = false

        try await store.saveSkill(mine)
        try await store.saveSkill(undo)

        let loaded = try await store.loadSkills()
        XCTAssertEqual(loaded.map(\.id), Skill.builtIn.map(\.id) + [mine.id])
        XCTAssertEqual(loaded.last, mine)
        XCTAssertEqual(loaded.first { $0.id == "undo" }?.isOn, false)
    }

    func testSkillsLeftOnThisDeviceMoveToICloudAndDeleteFromBoth() async throws {
        let (store, synced, local) = try store()
        let mine = try skill("Weekly")
        let context = ModelContext(local.container)
        let old = StoredSkill(id: mine.id)
        old.updatedAt = now
        old.skill = try Blob.encode(mine)
        context.insert(old)
        try context.save()

        let loaded = try await store.loadSkills()
        XCTAssertEqual(loaded.last, mine)
        XCTAssertEqual(try synced.skillCount(), 1)
        XCTAssertEqual(try local.skillCount(), 0)

        try await store.deleteSkills([mine.id])
        XCTAssertEqual(try synced.skillCount(), 0)
        let afterDelete = try await store.loadSkills()
        XCTAssertEqual(afterDelete.map(\.id), Skill.builtIn.map(\.id))
    }

    func testMovesSyncedConversationsAndSkillsOutOfTheRecordsStore() async throws {
        let records = try LiveStore.inMemory()
        let shared = try ConversationStore.localInMemory()
        var chat = conversation("Plan the week")
        chat.isSynced = true
        let mine = try skill("Weekly")
        try await ConversationStore(synced: records, local: try ConversationStore.localInMemory()).save(chat)
        try await ConversationStore(synced: records, local: try ConversationStore.localInMemory()).saveSkill(mine)

        try ConversationStore.move(from: records, to: shared)
        try ConversationStore.move(from: records, to: shared)

        XCTAssertEqual(try ModelContainerProbe(records).count(), 0)
        XCTAssertEqual(try ModelContainerProbe(records).skillCount(), 0)
        let store = ConversationStore(synced: shared, local: try ConversationStore.localInMemory())
        let loaded = try await store.load()
        XCTAssertEqual(loaded, [chat])
        let skills = try await store.loadSkills()
        XCTAssertEqual(skills.last, mine)
    }
}

/// Counts the conversations a container holds.
struct ModelContainerProbe {
    let container: ModelContainer

    init(_ container: ModelContainer) { self.container = container }

    func count() throws -> Int {
        try ModelContext(container).fetchCount(FetchDescriptor<StoredConversation>())
    }

    func conversations() throws -> [StoredConversation] {
        try ModelContext(container).fetch(FetchDescriptor<StoredConversation>())
    }

    func skillCount() throws -> Int {
        try ModelContext(container).fetchCount(FetchDescriptor<StoredSkill>())
    }
}
