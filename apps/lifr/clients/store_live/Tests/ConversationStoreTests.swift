import Foundation
import LifrAssistantClient
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
}

/// Counts the conversations a container holds.
struct ModelContainerProbe {
    let container: ModelContainer

    init(_ container: ModelContainer) { self.container = container }

    func count() throws -> Int {
        try ModelContext(container).fetchCount(FetchDescriptor<StoredConversation>())
    }
}
