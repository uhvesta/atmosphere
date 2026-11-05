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
        _ workspace: Workspace, clock: TestClock<Duration> = TestClock(),
        prewarmed: LockIsolated<Int> = LockIsolated(0),
        answer: @escaping @Sendable (AssistantRequest) async throws -> AssistantReply
    ) -> TestStoreOf<ChatFeature> {
        let store = TestStore(
            initialState: ChatFeature.State(
                conversation: Conversation(id: conversationID, createdAt: now), workspace: workspace)
        ) {
            ChatFeature()
        } withDependencies: { [now] in
            $0.store = .inMemory(workspace, actor: .user(device: "iPhone", surface: .iPhone))
            $0.skills = .inMemory()
            $0.guidance = .inMemory()
            $0.assistant.availability = { .available }
            $0.assistant.prewarm = { prewarmed.withValue { $0 += 1 } }
            $0.continuousClock = clock
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
        await store.skipReceivedActions(strict: false)
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

    func testEditingTheDraftChecksItAndAddAddsWhatTheCardShows() async throws {
        let (workspace, _) = home()
        let plan = Plan(tasks: [Plan.TaskDraft(title: "Buy soil", place: "HOME")])
        let store = chat(workspace) { _ in AssistantReply(text: "Here's a plan.", plan: plan, isComplete: true) }
        await send("Plan repotting", in: store)
        XCTAssertTrue(store.state.canAdd)

        var typo = plan
        typo.tasks[0].place = "HOEM"
        await store.send(.binding(.set(\.conversation.draft, typo)))
        XCTAssertNotNil(store.state.problem)
        await store.send(.addTapped)
        XCTAssertFalse(store.state.workspace.tasks.values.contains { $0.title == "Buy soil" })

        var edited = plan
        edited.tasks[0].title = "Buy potting soil"
        await store.send(.binding(.set(\.conversation.draft, edited)))
        XCTAssertNil(store.state.problem)
        await store.send(.addTapped)
        await store.skipReceivedActions()
        XCTAssertTrue(store.state.workspace.tasks.values.contains { $0.title == "Buy potting soil" })
        XCTAssertFalse(store.state.workspace.tasks.values.contains { $0.title == "Buy soil" })
    }

    func testRemovingEveryRowDiscardsTheDraft() async throws {
        let (workspace, _) = home()
        let plan = Plan(tasks: [Plan.TaskDraft(title: "Buy soil", place: "HOME")])
        let store = chat(workspace) { _ in AssistantReply(text: "Here's a plan.", plan: plan, isComplete: true) }
        await send("Plan repotting", in: store)

        await store.send(.binding(.set(\.conversation.draft, Plan())))
        XCTAssertNil(store.state.conversation.draft)
        XCTAssertNil(store.state.problem)
    }

    func testASuggestionInAnEmptyChatIsSentAsTheMessage() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "Here's your week.", isComplete: true) }
        await store.send(.task)

        await store.send(.suggestionTapped("Plan my week"))
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(store.state.conversation.messages.map(\.text), ["Plan my week", "Here's your week."])
    }

    func testASuggestionDoesNothingOnceTheChatHasMessages() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "Done.", isComplete: true) }
        await send("Hello", in: store)

        await store.send(.suggestionTapped("Plan my week"))
        await store.skipReceivedActions(strict: false)

        XCTAssertFalse(store.state.conversation.messages.contains { $0.text == "Plan my week" })
    }

    func testEditOpensOnlyWithADraftAndClosesWhenItsAdded() async throws {
        let (workspace, _) = home()
        let plan = Plan(tasks: [Plan.TaskDraft(title: "Buy soil", place: "HOME")])
        let store = chat(workspace) { _ in AssistantReply(text: "Here's a plan.", plan: plan, isComplete: true) }
        await store.send(.task)
        await store.send(.editDraftTapped)
        XCTAssertFalse(store.state.editsDraft)

        await send("Plan repotting", in: store)
        await store.send(.editDraftTapped)
        XCTAssertTrue(store.state.editsDraft)

        await store.send(.addTapped)
        await store.skipReceivedActions()
        XCTAssertFalse(store.state.editsDraft)
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
        let store = chat(workspace) { _ in throw AssistantError.tooLong(saved: false) }

        await send("Hello", in: store)
        XCTAssertNil(store.state.replying)
        XCTAssertNotNil(store.state.alert)
        XCTAssertEqual(store.state.conversation.messages.map(\.text), ["Hello"])
    }

    func testTryAgainSendsAnUnreadableReplysMessageAgain() async throws {
        let (workspace, _) = home()
        let attempts = LockIsolated(0)
        let store = chat(workspace) { _ in
            let attempt = attempts.withValue {
                $0 += 1
                return $0
            }
            if attempt == 1 { throw AssistantError.unreadable(saved: false) }
            return AssistantReply(text: "Done.", isComplete: true)
        }

        await send("Hello", in: store)
        XCTAssertNotNil(store.state.alert)
        XCTAssertNotNil(store.state.failed)

        await store.send(.alert(.presented(.tryAgain)))
        await store.skipReceivedActions()
        XCTAssertNil(store.state.alert)
        XCTAssertNil(store.state.failed)
        XCTAssertEqual(store.state.conversation.messages.map(\.text), ["Hello", "Done."])
    }

    func testChangesSavedBeforeAnUnreadableReplyAreNotMadeAgain() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in throw AssistantError.unreadable(saved: true) }

        await send("Hello", in: store)
        XCTAssertNotNil(store.state.alert)
        XCTAssertNil(store.state.failed)
    }

    func testOpeningTheChatLoadsTheModel() async {
        let (workspace, _) = home()
        let prewarmed = LockIsolated(0)
        let store = chat(workspace, prewarmed: prewarmed) { _ in AssistantReply(text: "", isComplete: true) }

        await store.send(.task)

        XCTAssertEqual(prewarmed.value, 1)
    }

    func testAReplyThatStallsTimesOutAndCanBeTriedAgain() async throws {
        let (workspace, _) = home()
        let clock = TestClock()
        let store = chat(workspace, clock: clock) { _ in
            try await Task.sleep(for: .seconds(3_600))
            return AssistantReply(text: "Late.", isComplete: true)
        }

        await send("Hello", in: store)
        XCTAssertNil(store.state.alert)

        await clock.advance(by: ChatFeature.stallLimit * 2)
        await store.receive(\.replyFailed)
        XCTAssertNil(store.state.replying)
        XCTAssertNotNil(store.state.alert)
        XCTAssertNotNil(store.state.failed)
        await store.send(.alert(.dismiss))
    }

    func testAReplyThatStallsAfterSavingIsNotTriedAgain() async throws {
        let (workspace, rent) = home()
        let clock = TestClock()
        let code = workspace.displayCode(of: rent)
        let store = chat(workspace, clock: clock) { request in
            _ = await request.tools().updateTasks(codes: [code], status: "done", priority: nil, due: nil, title: nil)
            try await Task.sleep(for: .seconds(3_600))
            return AssistantReply(text: "Late.", isComplete: true)
        }

        await send("Finish rent", in: store)
        await clock.advance(by: ChatFeature.stallLimit * 2)
        await store.receive(\.replyFailed)
        XCTAssertNotNil(store.state.alert)
        XCTAssertNil(store.state.failed)
        await store.send(.alert(.dismiss))
    }

    func testAReplyThatKeepsWritingDoesntTimeOut() async throws {
        let (workspace, _) = home()
        let clock = TestClock()
        let store = chat(workspace, clock: clock) { _ in AssistantReply(text: "", isComplete: false) }
        store.dependencies.assistant.reply = { _ in
            AssistantReplies { continuation in
                Task {
                    for step in 1...3 {
                        try await clock.sleep(for: ChatFeature.stallLimit / 2)
                        continuation.yield(AssistantReply(text: "Part \(step)"))
                    }
                    continuation.yield(AssistantReply(text: "Done.", isComplete: true))
                    continuation.finish()
                }
            }
        }

        await send("Hello", in: store)
        for _ in 1...3 { await clock.advance(by: ChatFeature.stallLimit / 2) }
        await store.skipReceivedActions()

        XCTAssertNil(store.state.alert)
        XCTAssertEqual(store.state.conversation.messages.map(\.text), ["Hello", "Done."])
    }

    func testABusyReplyKeepsItsTextAndIsSentAgainOnResume() async throws {
        let (workspace, _) = home()
        let attempts = LockIsolated(0)
        let store = chat(workspace) { _ in
            let attempt = attempts.withValue {
                $0 += 1
                return $0
            }
            if attempt == 1 { throw AssistantError.busy }
            return AssistantReply(text: "Done.", isComplete: true)
        }

        await send("Hello", in: store)
        XCTAssertNotNil(store.state.replying)
        XCTAssertNotNil(store.state.waiting)
        XCTAssertNil(store.state.alert)
        XCTAssertFalse(store.state.canSend)

        await store.send(.resume)
        await store.skipReceivedActions()
        XCTAssertNil(store.state.waiting)
        XCTAssertEqual(store.state.conversation.messages.map(\.text), ["Hello", "Done."])
    }

    func testTheAssistantIsToldThePersonsInstructions() async throws {
        let (workspace, _) = home()
        let told = LockIsolated<[String]>([])
        let store = chat(workspace) { request in
            told.withValue { $0.append(request.guidance) }
            return AssistantReply(text: "Arr.", isComplete: true)
        }
        await send("Hello", in: store)

        var guidance = Guidance()
        try guidance.save("Answer like a pirate.", id: UUID(), at: now)
        store.dependencies.guidance = .inMemory(guidance)
        await store.send(.binding(.set(\.input, "Hello again")))
        await store.send(.sendTapped)
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(told.value, [Guidance.defaultText, "Answer like a pirate."])
    }

    func testThePickedSkillGoesWithOneMessageOnly() async throws {
        let (workspace, _) = home()
        let sent = LockIsolated<[Skill?]>([])
        let store = chat(workspace) { request in
            sent.withValue { $0.append(request.skill) }
            return AssistantReply(text: "Sure.", isComplete: true)
        }

        await store.send(.task)
        await store.skipReceivedActions()
        await store.send(.findSkillTapped)
        await store.send(.binding(.set(\.skillQuery, "ti")))
        XCTAssertEqual(store.state.skillChoices.first?.id, "tidy")
        await store.send(.pickSkillTapped("tidy"))
        XCTAssertFalse(store.state.findsSkill)
        await send("Only HOME", in: store)
        XCTAssertNil(store.state.picked)
        await send("Thanks", in: store)

        XCTAssertEqual(sent.value, [.tidy, nil])
        XCTAssertEqual(store.state.conversation.messages.map(\.skill), ["tidy", nil, nil, nil])
    }

    func testASkillCanBeSentWithoutText() async {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "Here's your week.", isComplete: true) }

        XCTAssertFalse(store.state.canSend)
        await store.send(.pickSkillTapped("review"))
        XCTAssertTrue(store.state.canSend)
        await send("", in: store)
        XCTAssertEqual(store.state.conversation.messages.first?.skill, "review")
    }

    func testASkillThatsOffCantBePicked() async {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "", isComplete: true) }
        await store.send(.pickSkillTapped("query"))
        var off = Skill.query
        off.isOn = false
        await store.send(.skillsLoaded([off, .tidy]))

        XCTAssertNil(store.state.picked)
        XCTAssertEqual(store.state.skillChoices.map(\.id), ["tidy"])
        await store.send(.pickSkillTapped("query"))
        XCTAssertNil(store.state.picked)
    }

    private func list(_ kept: ConversationsClient) -> TestStoreOf<ConversationsFeature> {
        let (workspace, _) = home()
        let store = TestStore(initialState: ConversationsFeature.State()) {
            ConversationsFeature()
        } withDependencies: { [now] in
            $0.store = .inMemory(workspace)
            $0.conversations = kept
            $0.skills = .inMemory()
            $0.guidance = .inMemory()
            $0.assistant = .scripted([AssistantReply(text: "Hi.", isComplete: true)])
            $0.continuousClock = TestClock()
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
        let id = try XCTUnwrap(store.state.openID)
        await store.send(.chats(.element(id: id, action: .task)))
        await store.send(.chats(.element(id: id, action: .binding(.set(\.input, "Hello")))))
        await store.send(.chats(.element(id: id, action: .sendTapped)))
        await store.skipReceivedActions()
        XCTAssertEqual(store.state.conversations.map(\.title), ["Hello"])
        let saved = try await kept.load()
        XCTAssertEqual(saved.map(\.messages.count), [2])

        let reopened = list(kept)
        await reopened.send(.task)
        await reopened.skipReceivedActions()
        XCTAssertEqual(reopened.state.conversations.map(\.title), ["Hello"])
    }

    func testAReplyKeepsGoingAfterLeavingTheChat() async throws {
        let kept = ConversationsClient.inMemory()
        let gate = AsyncStream.makeStream(of: Void.self)
        let store = list(kept)
        store.dependencies.assistant.reply = { _ in
            AssistantReplies { continuation in
                Task {
                    for await _ in gate.stream { break }
                    continuation.yield(AssistantReply(text: "Later.", isComplete: true))
                    continuation.finish()
                }
            }
        }

        await store.send(.newTapped)
        let id = try XCTUnwrap(store.state.openID)
        await store.send(.chats(.element(id: id, action: .binding(.set(\.input, "Plan my week")))))
        await store.send(.chats(.element(id: id, action: .sendTapped)))
        await store.send(.closed)
        XCTAssertNil(store.state.openID)
        XCTAssertNotNil(store.state.chats[id: id], "a chat that's replying is kept")

        gate.continuation.yield()
        await store.skipReceivedActions()
        XCTAssertEqual(store.state.conversations[id: id]?.messages.map(\.text), ["Plan my week", "Later."])
        XCTAssertTrue(store.state.chats.isEmpty, "a finished chat off screen is let go")
        let saved = try await kept.load()
        XCTAssertEqual(saved.first?.messages.count, 2)
    }

    func testOpeningAnotherChatDoesntStopAReply() async throws {
        let kept = ConversationsClient.inMemory()
        let gate = AsyncStream.makeStream(of: Void.self)
        let store = list(kept)
        store.dependencies.assistant.reply = { _ in
            AssistantReplies { continuation in
                Task {
                    for await _ in gate.stream { break }
                    continuation.yield(AssistantReply(text: "Later.", isComplete: true))
                    continuation.finish()
                }
            }
        }

        await store.send(.newTapped)
        let first = try XCTUnwrap(store.state.openID)
        await store.send(.chats(.element(id: first, action: .binding(.set(\.input, "First")))))
        await store.send(.chats(.element(id: first, action: .sendTapped)))
        await store.send(.newTapped)
        XCTAssertNotEqual(store.state.openID, first)

        gate.continuation.yield()
        await store.skipReceivedActions()
        XCTAssertEqual(store.state.conversations[id: first]?.messages.map(\.text), ["First", "Later."])
    }

    private func chat(_ text: String) -> Conversation {
        var conversation = Conversation(id: UUID(), createdAt: now)
        conversation.messages = [ChatMessage(id: UUID(), role: .person, text: text, at: now)]
        return conversation
    }

    func testArchivingMovesAConversationOutOfTheListAndBack() async throws {
        let first = chat("First")
        let second = chat("Second")
        let kept = ConversationsClient.inMemory([first, second])
        let store = list(kept)
        await store.send(.task)
        await store.skipReceivedActions()

        await store.send(.archiveTapped(first.id))
        await store.finish()
        XCTAssertEqual(store.state.sorted.map(\.id), [second.id])
        XCTAssertEqual(store.state.archived.map(\.id), [first.id])
        var saved = try await kept.load()
        XCTAssertEqual(saved.first { $0.id == first.id }?.archivedAt, now)

        await store.send(.unarchiveTapped(first.id))
        await store.finish()
        XCTAssertEqual(Set(store.state.sorted.map(\.id)), [first.id, second.id])
        saved = try await kept.load()
        XCTAssertNil(saved.first { $0.id == first.id }?.archivedAt)
    }

    func testArchivingTheOpenChatClosesIt() async throws {
        let first = chat("First")
        let store = list(.inMemory([first]))
        await store.send(.task)
        await store.skipReceivedActions()
        await store.send(.opened(first.id))

        await store.send(.archiveTapped(first.id))
        XCTAssertNil(store.state.openID)
        XCTAssertTrue(store.state.chats.isEmpty)
    }

    func testSyncingIsPerConversationAndKeptWithIt() async throws {
        let first = chat("First")
        let second = chat("Second")
        let kept = ConversationsClient.inMemory([first, second])
        let store = list(kept)
        await store.send(.task)
        await store.skipReceivedActions()
        await store.send(.opened(first.id))

        await store.send(.syncTapped(first.id))
        await store.finish()
        XCTAssertEqual(store.state.chat?.conversation.isSynced, true)
        var saved = try await kept.load()
        XCTAssertEqual(saved.first { $0.id == first.id }?.isSynced, true)
        XCTAssertEqual(saved.first { $0.id == second.id }?.isSynced, false)

        await store.send(.syncTapped(first.id))
        await store.finish()
        saved = try await kept.load()
        XCTAssertEqual(saved.first { $0.id == first.id }?.isSynced, false)
    }

    func testDeletingArchivedConversationsAsksFirstAndKeepsTheTasks() async throws {
        let (workspace, _) = home()
        var first = chat("First")
        first.archivedAt = now
        var second = chat("Second")
        second.archivedAt = now
        let third = chat("Third")
        let kept = ConversationsClient.inMemory([first, second, third])
        let store = list(kept)
        await store.send(.task)
        await store.skipReceivedActions()

        await store.send(.deleteTapped(first.id))
        var saved = try await kept.load()
        XCTAssertEqual(saved.count, 3)
        await store.send(.confirm(.presented(.delete(first.id))))
        await store.finish()
        saved = try await kept.load()
        XCTAssertEqual(Set(saved.map(\.id)), [second.id, third.id])

        await store.send(.deleteArchivedTapped)
        XCTAssertNotNil(store.state.confirm)
        await store.send(.confirm(.presented(.deleteArchived)))
        await store.finish()
        saved = try await kept.load()
        XCTAssertEqual(saved.map(\.id), [third.id])
        XCTAssertEqual(store.state.conversations.map(\.id), [third.id])
        let tasks = try await store.dependencies.store.load().tasks.count
        XCTAssertEqual(tasks, workspace.tasks.count)
    }

    func testRenamingKeepsTheNameAndAnEmptyOneGoesBackToTheFirstMessage() async throws {
        let first = chat("Plan the week")
        let kept = ConversationsClient.inMemory([first])
        let store = list(kept)
        await store.send(.task)
        await store.skipReceivedActions()

        await store.send(.renameTapped(first.id))
        await store.send(.rename(.presented(.binding(.set(\.name, "  Week plan ")))))
        await store.send(.rename(.presented(.saveTapped)))
        await store.receive(\.rename.presented.delegate.renamed)
        await store.finish()
        XCTAssertNil(store.state.rename)
        var saved = try await kept.load()
        XCTAssertEqual(saved.first?.title, "Week plan")
        XCTAssertEqual(saved.first?.isNamedByHand, true)

        await store.send(.renameTapped(first.id))
        XCTAssertEqual(store.state.rename?.name, "Week plan")
        await store.send(.rename(.presented(.binding(.set(\.name, "")))))
        await store.send(.rename(.presented(.saveTapped)))
        await store.skipReceivedActions()
        await store.finish()
        saved = try await kept.load()
        XCTAssertEqual(saved.first?.title, "Plan the week")
        XCTAssertEqual(saved.first?.isNamedByHand, false)
    }

    func testTheRenameSheetFillsInTheAssistantsSuggestionOrSaysWhyNot() async {
        let store = TestStore(initialState: RenameFeature.State(chat("Plan the week"))) {
            RenameFeature()
        } withDependencies: {
            $0.assistant.suggestTitle = { _ in "Weekly plan" }
        }
        store.exhaustivity = .off
        await store.send(.suggestTapped)
        await store.receive(\.suggested.success)
        XCTAssertEqual(store.state.name, "Weekly plan")

        store.dependencies.assistant.suggestTitle = { _ in throw AssistantError.busy }
        await store.send(.suggestTapped)
        await store.receive(\.suggested.failure)
        XCTAssertEqual(store.state.name, "Weekly plan")
        XCTAssertNotNil(store.state.problem)
        XCTAssertFalse(store.state.suggesting)
    }

    func testTheAssistantNamesANewConversationOnlyWhenTurnedOn() async throws {
        let store = list(.inMemory())
        await store.send(.newTapped)
        let id = try XCTUnwrap(store.state.openID)
        await store.send(.chats(.element(id: id, action: .binding(.set(\.input, "Plan the week")))))
        await store.send(.chats(.element(id: id, action: .sendTapped)))
        await store.skipReceivedActions()
        await store.finish()
        XCTAssertEqual(store.state.conversations[id: id]?.title, "Plan the week")

        let on = try XCTUnwrap(store.state.chats[id: id]).$namesConversations
        on.withLock { $0 = true }
        await store.send(.newTapped)
        let named = try XCTUnwrap(store.state.openID)
        await store.send(.chats(.element(id: named, action: .binding(.set(\.input, "Plan the week")))))
        await store.send(.chats(.element(id: named, action: .sendTapped)))
        await store.skipReceivedActions()
        await store.finish()
        XCTAssertEqual(store.state.conversations[id: named]?.title, "Planning the week")
        XCTAssertEqual(store.state.conversations[id: named]?.isNamedByHand, false)
    }

    func testTheAssistantNeverRenamesAConversationNamedByHand() async throws {
        let store = list(.inMemory())
        await store.send(.newTapped)
        let id = try XCTUnwrap(store.state.openID)
        try XCTUnwrap(store.state.chats[id: id]).$namesConversations.withLock { $0 = true }
        await store.send(.chats(.element(id: id, action: .renameTapped)))
        await store.skipReceivedActions()
        await store.send(.rename(.presented(.binding(.set(\.name, "Mine")))))
        await store.send(.rename(.presented(.saveTapped)))
        await store.skipReceivedActions()

        await store.send(.chats(.element(id: id, action: .binding(.set(\.input, "Plan the week")))))
        await store.send(.chats(.element(id: id, action: .sendTapped)))
        await store.skipReceivedActions()
        await store.finish()
        XCTAssertEqual(store.state.conversations[id: id]?.title, "Mine")
    }
}
