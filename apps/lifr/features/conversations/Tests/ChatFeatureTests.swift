import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrConversations
import LifrCore
import LifrStoreClient
import XCTest

@MainActor
final class ChatFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private let conversationID = UUID(0)

    private func home() -> (Workspace, LifrTask) {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        let rent = records.task("Pay rent", in: home)
        return (records.workspace, rent)
    }

    /// A chat whose assistant answers with `answer`, run with the request.
    private func chat(
        _ workspace: Workspace,
        answer: @escaping @Sendable (AssistantRequest) async throws -> AssistantReply
    ) -> TestStoreOf<ChatFeature> {
        let store = TestStore(
            initialState: ChatFeature.State(
                conversation: Conversation(id: conversationID, createdAt: now), workspace: workspace)
        ) {
            ChatFeature()
        } withDependencies: { [now] in
            $0.store = .inMemory(workspace, actor: .user(device: "iPhone", surface: .iPhone))
            $0.assistant.availability = { .available }
            $0.assistant.reply = { request in
                AssistantReplies { continuation in
                    Task {
                        do {
                            continuation.yield(try await answer(request))
                            continuation.finish()
                        } catch {
                            continuation.finish(throwing: error)
                        }
                    }
                }
            }
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = .incrementing
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
        }
        store.exhaustivity = .off
        return store
    }

    private func send(_ text: String, in store: TestStoreOf<ChatFeature>) async {
        await store.send(.task)
        await store.send(.binding(.set(\.input, text)))
        await store.send(.sendTapped)
        await store.skipReceivedActions()
    }

    func testAddingTheDraftSavesItsTasksAndClearsTheCard() async throws {
        let (workspace, _) = home()
        let plan = Plan(tasks: [Plan.TaskDraft(title: "Buy soil", place: "HOME")])
        let store = chat(workspace) { _ in AssistantReply(text: "Here's a plan.", plan: plan, isComplete: true) }

        await send("Plan repotting", in: store)
        XCTAssertEqual(store.state.conversation.draft, plan)
        XCTAssertEqual(store.state.conversation.messages.map(\.role), [.person, .assistant])

        await store.send(.addTapped)
        await store.skipReceivedActions()
        XCTAssertNil(store.state.conversation.draft)
        XCTAssertTrue(store.state.workspace.tasks.values.contains { $0.title == "Buy soil" })
        let entry = try XCTUnwrap(store.state.conversation.added.first)
        XCTAssertEqual(
            store.state.workspace.journal.first { $0.id == entry }?.conversation, conversationID)
    }

    func testUndoPutsBackWhatTheAssistantChangedInThatReply() async throws {
        let (workspace, rent) = home()
        let code = workspace.displayCode(of: rent)
        let store = chat(workspace) { request in
            _ = await request.tools().updateTasks(codes: [code], status: "done", priority: nil, due: nil, title: nil)
            return AssistantReply(text: "Marked it done.", isComplete: true)
        }

        await send("I paid the rent", in: store)
        XCTAssertEqual(store.state.workspace.tasks[rent.id]?.status, .done)
        let reply = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertNotNil(store.state.conversation.changes[reply.id])

        await store.send(.undoTapped(messageID: reply.id))
        await store.skipReceivedActions()
        XCTAssertEqual(store.state.workspace.tasks[rent.id]?.status, .todo)
        XCTAssertNil(store.state.conversation.changes[reply.id])
    }

    func testAFailedReplyKeepsTheMessageAndSaysWhy() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in throw AssistantError.tooLong }

        await send("Hello", in: store)
        XCTAssertNil(store.state.replying)
        XCTAssertNotNil(store.state.alert)
        XCTAssertEqual(store.state.conversation.messages.map(\.text), ["Hello"])
    }

    private func list(_ kept: ConversationsClient) -> TestStoreOf<ConversationsFeature> {
        let (workspace, _) = home()
        let store = TestStore(initialState: ConversationsFeature.State()) {
            ConversationsFeature()
        } withDependencies: { [now] in
            $0.store = .inMemory(workspace)
            $0.conversations = kept
            $0.assistant = .scripted([AssistantReply(text: "Hi.", isComplete: true)])
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return store
    }

    func testTheListKeepsChatsThatHaveMessages() async throws {
        let kept = ConversationsClient.inMemory()
        let store = list(kept)

        await store.send(.task)
        await store.send(.newTapped)
        XCTAssertTrue(store.state.conversations.isEmpty)
        await store.send(.chat(.presented(.task)))
        await store.send(.chat(.presented(.binding(.set(\.input, "Hello")))))
        await store.send(.chat(.presented(.sendTapped)))
        await store.skipReceivedActions()
        XCTAssertEqual(store.state.conversations.map(\.title), ["Hello"])
        let saved = try await kept.load()
        XCTAssertEqual(saved.map(\.messages.count), [2])

        let reopened = list(kept)
        await reopened.send(.task)
        await reopened.skipReceivedActions()
        XCTAssertEqual(reopened.state.conversations.map(\.title), ["Hello"])
    }

    func testDeletingConversationsKeepsTheTasks() async throws {
        let (workspace, _) = home()
        func chat(_ text: String) -> Conversation {
            var conversation = Conversation(id: UUID(), createdAt: now)
            conversation.messages = [ChatMessage(id: UUID(), role: .person, text: text, at: now)]
            return conversation
        }
        let first = chat("First")
        let second = chat("Second")
        let kept = ConversationsClient.inMemory([first, second])
        let store = list(kept)
        await store.send(.task)
        await store.skipReceivedActions()

        await store.send(.deleteTapped(first.id))
        await store.finish()
        var saved = try await kept.load()
        XCTAssertEqual(saved.map(\.id), [second.id])

        await store.send(.deleteAllTapped)
        await store.send(.confirm(.presented(.deleteAll)))
        await store.finish()
        saved = try await kept.load()
        XCTAssertTrue(saved.isEmpty)
        XCTAssertTrue(store.state.conversations.isEmpty)
        let tasks = try await store.dependencies.store.load().tasks.count
        XCTAssertEqual(tasks, workspace.tasks.count)
    }
}
