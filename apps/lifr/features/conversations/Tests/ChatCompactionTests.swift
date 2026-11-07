import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrConversations
import LifrCore
import LifrStoreClient
import XCTest

@MainActor
final class ChatCompactionTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    /// A chat whose replies say `leftOut` messages didn't fit, and whose
    /// summaries are `summary`, keeping what each request and summary got.
    private func chat(
        leftOut: Int, summary: @escaping @Sendable () throws -> String = { "They planned the week." },
        asked: LockIsolated<[AssistantRequest]> = LockIsolated([]),
        compacted: LockIsolated<[([ChatMessage], String?)]> = LockIsolated([])
    ) -> TestStoreOf<ChatFeature> {
        let store = TestStore(
            initialState: ChatFeature.State(
                conversation: Conversation(id: UUID(0), createdAt: now), workspace: Workspace())
        ) {
            ChatFeature()
        } withDependencies: { [now] in
            $0.store = .inMemory(Workspace(), actor: .user(device: "iPhone", surface: .iPhone))
            $0.skills = .inMemory()
            $0.guidance = .inMemory()
            $0.assistant.availability = { .available }
            $0.assistant.pickSkill = { _, _ in nil }
            $0.assistant.prewarm = {}
            $0.assistant.inputs = { [] }
            $0.assistant.reply = { request in
                asked.withValue { $0.append(request) }
                return AssistantReplies { continuation in
                    continuation.yield(AssistantReply(text: "Noted.", isComplete: true, leftOut: leftOut))
                    continuation.finish()
                }
            }
            $0.assistant.compact = { messages, earlier in
                compacted.withValue { $0.append((messages, earlier)) }
                return try summary()
            }
            $0.continuousClock = TestClock()
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return store
    }

    private func send(_ texts: [String], in store: TestStoreOf<ChatFeature>) async {
        await store.send(.task)
        for text in texts {
            await store.send(.binding(.set(\.input, text)))
            await store.send(.sendTapped)
            await store.skipReceivedActions(strict: false)
        }
    }

    func testAReplyWrittenWithoutOlderMessagesOffersCompact() async {
        let store = chat(leftOut: 0)
        await send(["Plan the week", "Add laundry"], in: store)
        XCTAssertFalse(store.state.offersCompact)

        let left = chat(leftOut: 2)
        await send(["Plan the week", "Add laundry"], in: left)
        XCTAssertTrue(left.state.offersCompact)
    }

    func testCompactSummarizesAllButTheLatestExchangeAndSendsTheSummaryInstead() async throws {
        let asked = LockIsolated<[AssistantRequest]>([])
        let compacted = LockIsolated<[([ChatMessage], String?)]>([])
        let store = chat(leftOut: 2, asked: asked, compacted: compacted)
        await send(["Plan the week", "Add laundry"], in: store)

        await store.send(.compactTapped)
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(compacted.value.first?.0.map(\.text), ["Plan the week", "Noted."])
        XCTAssertNil(compacted.value.first?.1)
        XCTAssertFalse(store.state.offersCompact)
        XCTAssertFalse(store.state.compacting)

        await send(["And groceries"], in: store)
        let last = try XCTUnwrap(asked.value.last)
        XCTAssertEqual(last.summary, "They planned the week.")
        XCTAssertEqual(last.messages.map(\.text), ["Add laundry", "Noted.", "And groceries"])

        await store.send(.compactTapped)
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(compacted.value.last?.0.map(\.text), ["Add laundry", "Noted."])
        XCTAssertEqual(compacted.value.last?.1, "They planned the week.")
    }

    func testCompactingByItselfWhenTurnedOn() async {
        let compacted = LockIsolated<[([ChatMessage], String?)]>([])
        let store = chat(leftOut: 2, compacted: compacted)
        await send(["Plan the week", "Add laundry"], in: store)
        XCTAssertTrue(compacted.value.isEmpty)

        store.state.$compactsConversations.withLock { $0 = true }
        await send(["And groceries"], in: store)

        XCTAssertEqual(compacted.value.count, 1)
        XCTAssertNotNil(store.state.conversation.summary)
    }

    func testForgettingTheSummarySendsEveryMessageAgain() async throws {
        let asked = LockIsolated<[AssistantRequest]>([])
        let store = chat(leftOut: 2, asked: asked)
        await send(["Plan the week", "Add laundry"], in: store)
        await store.send(.compactTapped)
        await store.skipReceivedActions(strict: false)
        let through = try XCTUnwrap(store.state.conversation.summary?.through)

        await store.send(.forgetSummaryTapped(through))
        await send(["And groceries"], in: store)

        XCTAssertNil(asked.value.last?.summary)
        XCTAssertEqual(asked.value.last?.messages.count, 5)
    }

    func testAFailedCompactSaysSoAndChangesNothing() async {
        struct Failed: Error {}
        let store = chat(leftOut: 2, summary: { throw Failed() })
        await send(["Plan the week", "Add laundry"], in: store)

        await store.send(.compactTapped)
        await store.skipReceivedActions(strict: false)

        XCTAssertNotNil(store.state.alert)
        XCTAssertTrue(store.state.conversation.summaries.isEmpty)
        XCTAssertTrue(store.state.offersCompact)
    }

    func testNothingToCompactInTheFirstExchange() async {
        let store = chat(leftOut: 1)
        await send(["Plan the week"], in: store)

        XCTAssertFalse(store.state.canCompact)
        await store.send(.compactTapped)
        XCTAssertFalse(store.state.compacting)
    }
}
