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
        let defaults = try XCTUnwrap(UserDefaults(suiteName: UUID().uuidString))
        return (
            ConversationStore(synced: synced, local: local, defaults: defaults),
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

    func testKeepsConversationsOnThisDeviceUntilSyncingIsOn() async throws {
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

    func testTurningSyncingOnMovesConversationsAndOffKeepsACopy() async throws {
        let (store, synced, local) = try store()
        let chat = conversation("Hello")
        try await store.save(chat)

        try await store.setSyncing(true)
        XCTAssertTrue(store.isSyncing)
        XCTAssertEqual(try synced.count(), 1)
        XCTAssertEqual(try local.count(), 0)
        let afterOn = try await store.load()
        XCTAssertEqual(afterOn, [chat])

        try await store.setSyncing(false)
        XCTAssertEqual(try local.count(), 1)
        XCTAssertEqual(try synced.count(), 1)
        let afterOff = try await store.load()
        XCTAssertEqual(afterOff, [chat])
    }

    func testDeletingRemovesTheConversationFromBothStores() async throws {
        let (store, synced, local) = try store()
        let kept = conversation("Keep")
        let gone = conversation("Gone")
        try await store.save(gone)
        try await store.setSyncing(true)
        try await store.setSyncing(false)
        try await store.save(kept)

        try await store.delete([gone.id])
        let loaded = try await store.load()
        XCTAssertEqual(loaded, [kept])
        XCTAssertEqual(try synced.count(), 0)

        try await store.deleteAll()
        XCTAssertEqual(try local.count(), 0)
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

    func testSkillsMoveWithSyncingAndDeleteFromBothStores() async throws {
        let (store, synced, local) = try store()
        let mine = try skill("Weekly")
        try await store.saveSkill(mine, now: now)

        try await store.setSyncing(true)
        XCTAssertEqual(try synced.skillCount(), 1)
        XCTAssertEqual(try local.skillCount(), 0)
        var edited = mine
        edited.instructions = "Be briefer."
        try await store.saveSkill(edited, now: now.addingTimeInterval(60))

        try await store.setSyncing(false)
        let afterOff = try await store.loadSkills()
        XCTAssertEqual(afterOff.last, edited)

        try await store.deleteSkills([mine.id])
        XCTAssertEqual(try synced.skillCount(), 0)
        XCTAssertEqual(try local.skillCount(), 0)
        let afterDelete = try await store.loadSkills()
        XCTAssertEqual(afterDelete.map(\.id), Skill.builtIn.map(\.id))
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
