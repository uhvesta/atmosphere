import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrConversations
import LifrCore
import LifrModelsClient
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
        prewarmed: LockIsolated<Int> = LockIsolated(0), inputs: Set<ModelInput> = [],
        attachments: AttachmentsClient = .inMemory(), recorder: RecorderClient = .scripted(),
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
            $0.assistant.inputs = { inputs }
            $0.attachments = attachments
            $0.recorder = recorder
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

    func testAPhotoAndARecordingGoWithTheNextMessageToTheModel() async throws {
        let (workspace, _) = home()
        let asked = LockIsolated<AssistantRequest?>(nil)
        let store = chat(workspace, inputs: [.image, .audio]) { request in
            asked.setValue(request)
            return AssistantReply(text: "A fern.", isComplete: true)
        }
        await store.send(.task)

        await store.send(.photoPicked(Data([1])))
        await store.skipReceivedActions(strict: false)
        await store.send(.recordTapped)
        await store.skipReceivedActions(strict: false)
        XCTAssertTrue(store.state.recording)
        XCTAssertFalse(store.state.canSend)
        await store.send(.recordTapped)
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(store.state.pending.map(\.kind), [.image, .audio])
        XCTAssertEqual(store.state.pending.last?.seconds, 4)
        XCTAssertTrue(store.state.canSend)

        await store.send(.sendTapped)
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(store.state.pending, [])
        let sent = try XCTUnwrap(store.state.conversation.messages.first)
        XCTAssertEqual(sent.attachments.map(\.kind), [.image, .audio])
        XCTAssertEqual(asked.value?.attachments.map(\.kind), [.image, .audio])
        XCTAssertEqual(store.state.conversation.messages.last?.text, "A fern.")
    }

    func testAModelThatReadsOnlyTextOffersNoAttachments() async {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "", isComplete: true) }
        await store.send(.task)

        await store.send(.photoPicked(Data([1])))
        await store.send(.recordTapped)
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(store.state.pending, [])
        XCTAssertFalse(store.state.recording)
    }

    func testARemovedAttachmentIsntSentOrKept() async throws {
        let (workspace, _) = home()
        let attachments = AttachmentsClient.inMemory()
        let store = chat(workspace, inputs: [.image], attachments: attachments) { _ in
            AssistantReply(text: "", isComplete: true)
        }
        await store.send(.task)
        await store.send(.photoPicked(Data([1])))
        await store.skipReceivedActions(strict: false)
        let photo = try XCTUnwrap(store.state.pending.first)

        await store.send(.removeAttachmentTapped(photo.id))
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(store.state.pending, [])
        XCTAssertFalse(store.state.canSend)
        XCTAssertNil(attachments.file(photo, conversationID))
    }

    func testAPhotoThatCantBeReadSaysSoAndAttachesNothing() async {
        let (workspace, _) = home()
        let store = chat(workspace, inputs: [.image]) { _ in AssistantReply(text: "", isComplete: true) }
        await store.send(.task)

        await store.send(.photoPicked(Data()))
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(store.state.pending, [])
        XCTAssertFalse(store.state.attaching)
        XCTAssertNotNil(store.state.alert)
    }

    func testWithoutTheMicrophoneNothingIsRecorded() async {
        let (workspace, _) = home()
        var recorder = RecorderClient.scripted()
        recorder.start = { throw AttachmentError.microphoneDenied }
        let store = chat(workspace, inputs: [.audio], recorder: recorder) { _ in
            AssistantReply(text: "", isComplete: true)
        }
        await store.send(.task)

        await store.send(.recordTapped)
        await store.skipReceivedActions(strict: false)

        XCTAssertFalse(store.state.recording)
        XCTAssertNotNil(store.state.alert)
    }

    func testACancelledRecordingAttachesNothing() async {
        let (workspace, _) = home()
        let store = chat(workspace, inputs: [.audio]) { _ in AssistantReply(text: "", isComplete: true) }
        await store.send(.task)

        await store.send(.recordTapped)
        await store.skipReceivedActions(strict: false)
        await store.send(.cancelRecordingTapped)
        await store.skipReceivedActions(strict: false)

        XCTAssertFalse(store.state.recording)
        XCTAssertEqual(store.state.pending, [])
    }

    func testAttachmentsSentFromAnotherDeviceAreMarked() async {
        let (workspace, _) = home()
        var conversation = Conversation(id: conversationID, createdAt: now)
        let photo = ChatAttachment(id: UUID(9), kind: .image)
        conversation.messages = [ChatMessage(id: UUID(8), role: .person, text: "", at: now, attachments: [photo])]
        let store = chat(workspace) { _ in AssistantReply(text: "", isComplete: true) }
        await store.send(.binding(.set(\.conversation, conversation)))

        await store.send(.task)

        XCTAssertEqual(store.state.elsewhere, [photo.id])
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
        await edit(typo, in: store)
        XCTAssertNotNil(store.state.problem)
        await store.send(.addTapped)
        XCTAssertFalse(store.state.workspace.tasks.values.contains { $0.title == "Buy soil" })

        var edited = plan
        edited.tasks[0].title = "Buy potting soil"
        await edit(edited, in: store)
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

        await edit(Plan(), in: store)
        XCTAssertNil(store.state.conversation.draft)
        XCTAssertNil(store.state.problem)
    }

    func testADraftLeavesOutAnAvenueThisChatAlreadyAdded() async throws {
        let (workspace, _) = home()
        let reading = Plan.AvenueDraft(key: "READ", name: "Reading")
        let replies = LockIsolated([
            Plan(avenues: [reading]),
            Plan(avenues: [reading, .init(key: "COOK", name: "Cooking")]),
        ])
        let store = chat(workspace) { _ in
            let plan = replies.withValue { $0.removeFirst() }
            return AssistantReply(text: "Here's a plan.", plan: plan, isComplete: true)
        }
        await send("Add a Reading Avenue", in: store)
        await store.send(.addTapped)
        await store.skipReceivedActions()
        await send("And a Cooking one", in: store)
        XCTAssertEqual(store.state.conversation.draft?.avenues.map(\.key), ["COOK"])
        XCTAssertTrue(store.state.canAdd)
    }

    /// Changes the draft as the editor does: its fields write the whole
    /// conversation.
    private func edit(_ plan: Plan, in store: TestStoreOf<ChatFeature>) async {
        var conversation = store.state.conversation
        conversation.draft = plan
        await store.send(.binding(.set(\.conversation, conversation)))
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
            _ = try? await request.tools().change(TaskChange(code: code, status: "done"))
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

    func testAReplysThinkingIsKeptClosedAndOpensAndClosesWithItsChevron() async throws {
        let (workspace, _) = home()
        let thinking = ModelThinking(text: "Rent is due.", duration: .seconds(4), processor: .gpu)
        let store = chat(workspace) { _ in AssistantReply(text: "Paid.", isComplete: true, thinking: thinking) }

        await send("I paid the rent", in: store)
        let reply = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertEqual(reply.thinking, thinking)
        XCTAssertEqual(store.state.shownThinking, [])

        await store.send(.thinkingToggled(messageID: reply.id))
        XCTAssertEqual(store.state.shownThinking, [reply.id])
        await store.send(.thinkingToggled(messageID: reply.id))
        XCTAssertEqual(store.state.shownThinking, [])
    }

    func testThinkingOpenedWhileTheReplyIsWrittenStaysOpen() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "") }
        let thinking = ModelThinking(text: "Rent is due.", processor: .gpu)

        await store.send(.replied(AssistantReply(text: "", thinking: thinking)))
        XCTAssertEqual(store.state.replyingThinking, thinking)
        await store.send(.thinkingToggled(messageID: nil))
        await store.send(.replied(AssistantReply(text: "Paid.", isComplete: true, thinking: thinking)))

        let reply = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertEqual(store.state.shownThinking, [reply.id])
        XCTAssertNil(store.state.replyingThinking)
        XCTAssertFalse(store.state.showsReplyThinking)
    }

    func testAReplyWithoutThoughtsShowsNoThinking() async throws {
        let (workspace, _) = home()
        let empty = ModelThinking(text: "  ", duration: .seconds(1), processor: .gpu)
        let store = chat(workspace) { _ in AssistantReply(text: "Paid.", isComplete: true, thinking: empty) }

        await send("I paid the rent", in: store)

        XCTAssertNil(store.state.conversation.messages.last?.thinking)
    }

    func testAFailedReplyKeepsTheMessageAndSaysWhy() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in throw AssistantError.tooLong(saved: false) }

        await send("Hello", in: store)
        XCTAssertNil(store.state.replying)
        XCTAssertNotNil(store.state.failure)
        XCTAssertEqual(store.state.conversation.messages.map(\.text), ["Hello"])
        let hello = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertFalse(store.state.canRetry(hello), "Sending a message that's too long again can't help")
    }

    func testRetrySendsAnUnreadableReplysMessageAgain() async throws {
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
        let hello = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertTrue(store.state.canRetry(hello))

        await store.send(.retryTapped(messageID: hello.id))
        await store.skipReceivedActions(strict: false)
        XCTAssertNil(store.state.failure)
        XCTAssertEqual(store.state.conversation.messages.map(\.text), ["Hello", "Done."])
    }

    func testChangesSavedBeforeAnUnreadableReplyAreNotMadeAgain() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in throw AssistantError.unreadable(saved: true) }

        await send("Hello", in: store)
        let hello = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertNotNil(store.state.failure)
        XCTAssertFalse(store.state.canRetry(hello))
    }

    func testOpeningTheChatLoadsNoModelUntilThePersonWrites() async {
        let (workspace, _) = home()
        let prewarmed = LockIsolated(0)
        let store = chat(workspace, prewarmed: prewarmed) { _ in AssistantReply(text: "", isComplete: true) }
        store.exhaustivity = .off

        await store.send(.task)
        XCTAssertEqual(prewarmed.value, 0)

        await store.send(.binding(.set(\.input, "A")))
        await store.send(.binding(.set(\.input, "Ad")))
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
        let hello = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertNotNil(store.state.failure)
        XCTAssertTrue(store.state.canRetry(hello))
    }

    func testAReplyThatStallsAfterSavingIsNotTriedAgain() async throws {
        let (workspace, rent) = home()
        let clock = TestClock()
        let code = workspace.displayCode(of: rent)
        let store = chat(workspace, clock: clock) { request in
            _ = try? await request.tools().change(TaskChange(code: code, status: "done"))
            try await Task.sleep(for: .seconds(3_600))
            return AssistantReply(text: "Late.", isComplete: true)
        }

        await send("Finish rent", in: store)
        await clock.advance(by: ChatFeature.stallLimit * 2)
        await store.receive(\.replyFailed)
        let finish = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertNotNil(store.state.failure)
        XCTAssertFalse(store.state.canRetry(finish))
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

    func testAReplyWaitingInTheBackgroundDoesntTimeOutWhenLifrIsBack() async throws {
        let (workspace, _) = home()
        let clock = TestClock()
        let returns = LockIsolated(0)
        let finish = AsyncStream.makeStream(of: Void.self)
        let store = chat(workspace, clock: clock) { _ in AssistantReply(text: "", isComplete: false) }
        store.dependencies.backgroundTime.returns = { returns.value }
        store.dependencies.assistant.reply = { _ in
            AssistantReplies { continuation in
                Task {
                    // The model waits for Lifr to be back, then answers.
                    var wake = finish.stream.makeAsyncIterator()
                    _ = await wake.next()
                    continuation.yield(AssistantReply(text: "Done.", isComplete: true))
                    continuation.finish()
                }
            }
        }

        await send("Hello", in: store)
        // Away for longer than a stall, then back.
        returns.setValue(1)
        await clock.advance(by: ChatFeature.stallLimit)
        await store.send(.resume)
        finish.continuation.yield()
        await store.skipReceivedActions()

        XCTAssertNil(store.state.alert)
        XCTAssertEqual(store.state.conversation.messages.map(\.text), ["Hello", "Done."])
    }

    func testAReplyStillBeingWrittenIsntSentAgainOnResume() async throws {
        let (workspace, _) = home()
        let sent = LockIsolated(0)
        let finish = AsyncStream.makeStream(of: Void.self)
        let store = chat(workspace) { _ in AssistantReply(text: "", isComplete: false) }
        store.dependencies.assistant.reply = { _ in
            sent.withValue { $0 += 1 }
            return AssistantReplies { continuation in
                Task {
                    continuation.yield(AssistantReply(text: "Half"))
                    var wake = finish.stream.makeAsyncIterator()
                    _ = await wake.next()
                    continuation.yield(AssistantReply(text: "Half and the rest.", isComplete: true))
                    continuation.finish()
                }
            }
        }

        await send("Hello", in: store)
        await store.send(.resume)
        finish.continuation.yield()
        await store.skipReceivedActions()

        XCTAssertEqual(sent.value, 1)
        XCTAssertEqual(store.state.conversation.messages.map(\.text), ["Hello", "Half and the rest."])
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

    // MARK: - Checkpoints

    /// Gives each message a later time, as when the person writes them.
    private func ticking(_ store: TestStoreOf<ChatFeature>) {
        let seconds = LockIsolated(0.0)
        store.dependencies.date = DateGenerator { [now] in
            seconds.withValue { $0 += 1 }
            return now.addingTimeInterval(seconds.value)
        }
    }

    private func texts(_ store: TestStoreOf<ChatFeature>) -> [String] {
        store.state.conversation.messages.map(\.text)
    }

    func testGoingBackAndForwardKeepsWhatCameAfter() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { request in
            AssistantReply(text: "re " + (request.messages.last?.text ?? ""), isComplete: true)
        }
        ticking(store)
        await send("one", in: store)
        await send("two", in: store)

        await store.send(.backTapped)
        XCTAssertEqual(texts(store), ["one", "re one"])
        await store.send(.forwardTapped)
        XCTAssertEqual(texts(store), ["one", "re one", "two", "re two"])

        await store.send(.backTapped)
        await send("two again", in: store)
        XCTAssertEqual(texts(store), ["one", "re one", "two again", "re two again"])
        let again = try XCTUnwrap(store.state.conversation.messages.first { $0.text == "two again" })
        XCTAssertEqual(store.state.conversation.branches(at: again.id).count, 2)
        XCTAssertEqual(
            store.state.conversation.checkpoints.map(\.message.text), ["one", "two", "two again"])

        await store.send(.branchTapped(messageID: again.id, by: -1))
        XCTAssertEqual(texts(store), ["one", "re one", "two", "re two"])
        let two = try XCTUnwrap(store.state.conversation.messages.first { $0.text == "two" })
        await store.send(.branchTapped(messageID: two.id, by: 1))
        XCTAssertEqual(texts(store), ["one", "re one", "two again", "re two again"])
    }

    func testGoingBackPastChangesAsksWhetherToUndoThem() async throws {
        let (workspace, rent) = home()
        let code = workspace.displayCode(of: rent)
        let store = chat(workspace) { request in
            if request.messages.last?.text == "I paid the rent" {
                _ = try? await request.tools().change(TaskChange(code: code, status: "done"))
            }
            return AssistantReply(text: "OK.", isComplete: true)
        }
        ticking(store)
        await send("Hello", in: store)
        await send("I paid the rent", in: store)
        let start = try XCTUnwrap(store.state.conversation.messages.dropFirst().first?.id)

        await store.send(.goTapped(start))
        XCTAssertNotNil(store.state.alert, "Asks before going back past changes")
        XCTAssertEqual(store.state.conversation.messages.count, 4)
        await store.send(.alert(.presented(.go(to: start, undo: false))))
        XCTAssertEqual(store.state.conversation.messages.count, 2)
        XCTAssertEqual(store.state.workspace.tasks[rent.id]?.status, .done)

        await store.send(.forwardTapped)
        await store.send(.goTapped(start))
        await store.send(.alert(.presented(.go(to: start, undo: true))))
        await store.skipReceivedActions()
        XCTAssertEqual(store.state.conversation.messages.count, 2)
        XCTAssertEqual(store.state.workspace.tasks[rent.id]?.status, .todo)

        await store.send(.forwardTapped)
        XCTAssertNil(store.state.alert, "Nothing left to undo")
        XCTAssertEqual(store.state.conversation.messages.count, 4)
        XCTAssertEqual(store.state.workspace.tasks[rent.id]?.status, .todo, "Going forward changes nothing again")
    }

    func testNothingGoesBackWhileAReplyIsWritten() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "OK.", isComplete: true) }
        ticking(store)
        await send("one", in: store)
        await send("two", in: store)
        store.dependencies.assistant.reply = { _ in AssistantReplies { _ in } }

        await store.send(.binding(.set(\.input, "three")))
        await store.send(.sendTapped)
        await store.send(.backTapped)
        XCTAssertEqual(store.state.conversation.messages.last?.text, "three")
    }

    func testAForkFromBeforeTheMessageBeingAnsweredCanBeMadeMeanwhile() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "OK.", isComplete: true) }
        ticking(store)
        await send("one", in: store)
        store.dependencies.assistant.reply = { _ in AssistantReplies { _ in } }
        await store.send(.binding(.set(\.input, "two")))
        await store.send(.sendTapped)
        let messages = store.state.conversation.messages
        let reply = try XCTUnwrap(messages.dropFirst().first)
        let asked = try XCTUnwrap(messages.last)

        XCTAssertFalse(store.state.canFork(at: asked.id), "It has no reply yet to fork with")
        await store.send(.forkTapped(messageID: reply.id))
        await store.receive(\.forked)

        XCTAssertNotNil(store.state.replying, "The reply goes on being written")
        XCTAssertEqual(texts(store), ["one", "OK.", "two"])
    }

    func testAnEditedMessageIsABranchBesideTheOldWithANewReply() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { request in
            AssistantReply(text: "re " + (request.messages.last?.text ?? ""), isComplete: true)
        }
        ticking(store)
        await send("one", in: store)
        await send("two", in: store)
        let two = try XCTUnwrap(store.state.conversation.messages.first { $0.text == "two" })

        await store.send(.editTapped(messageID: two.id))
        XCTAssertEqual(store.state.input, "two")
        await send("two, edited", in: store)

        XCTAssertNil(store.state.editing)
        XCTAssertEqual(texts(store), ["one", "re one", "two, edited", "re two, edited"])
        let edited = try XCTUnwrap(store.state.conversation.messages.first { $0.text == "two, edited" })
        XCTAssertEqual(store.state.conversation.branches(at: edited.id).map(\.text), ["two", "two, edited"])
    }

    func testCancellingAnEditLeavesTheChatAsItWas() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "OK.", isComplete: true) }
        ticking(store)
        await send("one", in: store)
        let one = try XCTUnwrap(store.state.conversation.messages.first)

        await store.send(.editTapped(messageID: one.id))
        await store.send(.cancelEditTapped)

        XCTAssertNil(store.state.editing)
        XCTAssertEqual(store.state.input, "")
        XCTAssertEqual(texts(store), ["one", "OK."])
    }

    func testEditingAMessageBeforeChangesAsksWhetherToUndoThem() async throws {
        let (workspace, rent) = home()
        let code = workspace.displayCode(of: rent)
        let store = chat(workspace) { request in
            if request.messages.last?.text == "I paid the rent" {
                _ = try? await request.tools().change(TaskChange(code: code, status: "done"))
            }
            return AssistantReply(text: "OK.", isComplete: true)
        }
        ticking(store)
        await send("I paid the rent", in: store)
        let paid = try XCTUnwrap(store.state.conversation.messages.first)

        await store.send(.editTapped(messageID: paid.id))
        await store.send(.binding(.set(\.input, "Hello")))
        await store.send(.sendTapped)
        XCTAssertNotNil(store.state.alert, "Asks before hiding the reply that changed the rent")
        XCTAssertEqual(texts(store), ["I paid the rent", "OK."])

        await store.send(.alert(.presented(.resend(undo: true))))
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(texts(store), ["Hello", "OK."])
        XCTAssertEqual(store.state.workspace.tasks[rent.id]?.status, .todo)
    }

    func testRetryWritesAReplyAgainBesideTheOld() async throws {
        let (workspace, _) = home()
        let count = LockIsolated(0)
        let store = chat(workspace) { _ in
            count.withValue { $0 += 1 }
            return AssistantReply(text: "reply \(count.value)", isComplete: true)
        }
        ticking(store)
        await send("one", in: store)
        let reply = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertTrue(store.state.canRetry(reply))

        await store.send(.retryTapped(messageID: reply.id))
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(texts(store), ["one", "reply 2"])
        let again = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertEqual(store.state.conversation.branches(at: again.id).map(\.text), ["reply 1", "reply 2"])
    }

    func testAForkOpensAsANewConversationUpToThere() async throws {
        let kept = ConversationsClient.inMemory()
        let attachments = AttachmentsClient.inMemory()
        let store = list(kept, attachments: attachments)
        await store.send(.task)
        await store.send(.newTapped)
        let id = try XCTUnwrap(store.state.openID)
        let photo = try await attachments.save(Data([1]), .image, nil, id)
        await store.send(.chats(.element(id: id, action: .attached(.success(photo)))))
        for text in ["Plan the week", "And the weekend"] {
            await store.send(.chats(.element(id: id, action: .binding(.set(\.input, text)))))
            await store.send(.chats(.element(id: id, action: .sendTapped)))
            await store.skipReceivedActions()
        }
        let original = try XCTUnwrap(store.state.chat?.conversation)
        let reply = try XCTUnwrap(original.messages.dropFirst().first?.id)

        await store.send(.chats(.element(id: id, action: .forkTapped(messageID: reply))))
        await store.skipReceivedActions()

        let fork = try XCTUnwrap(store.state.chat?.conversation)
        XCTAssertNotEqual(fork.id, id)
        XCTAssertEqual(fork.messages.map(\.text), ["Plan the week", "Hi."])
        XCTAssertNotEqual(fork.title, original.title)
        XCTAssertNotNil(attachments.file(photo, fork.id), "The photo goes with the fork")
        XCTAssertEqual(store.state.conversations[id: id]?.messages.count, 4)
        let saved = try await kept.load()
        XCTAssertEqual(Set(saved.map(\.id)), [id, fork.id])
    }

    private func list(
        _ kept: ConversationsClient, attachments: AttachmentsClient = .inMemory()
    ) -> TestStoreOf<ConversationsFeature> {
        let (workspace, _) = home()
        let store = TestStore(initialState: ConversationsFeature.State()) {
            ConversationsFeature()
        } withDependencies: { [now] in
            $0.store = .inMemory(workspace)
            $0.conversations = kept
            $0.attachments = attachments
            $0.recorder = .scripted()
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

    func testLifrReopensTheChatThatWasOpenWithWhatWasTypedInIt() async throws {
        let kept = ConversationsClient.inMemory()
        let before = list(kept)
        await before.send(.newTapped)
        let id = try XCTUnwrap(before.state.openID)
        await before.send(.chats(.element(id: id, action: .binding(.set(\.input, "Hello")))))
        await before.send(.chats(.element(id: id, action: .sendTapped)))
        await before.skipReceivedActions()
        await before.send(.chats(.element(id: id, action: .binding(.set(\.input, "And then")))))

        let after = list(kept)
        await after.send(.task)
        await after.receive(\.loaded)

        XCTAssertEqual(after.state.openID, id)
        XCTAssertEqual(after.state.chat?.input, "And then")
    }

    func testAChatClosedBeforeLifrQuitStaysClosed() async throws {
        let kept = ConversationsClient.inMemory()
        let before = list(kept)
        await before.send(.newTapped)
        let id = try XCTUnwrap(before.state.openID)
        await before.send(.chats(.element(id: id, action: .binding(.set(\.input, "Hello")))))
        await before.send(.chats(.element(id: id, action: .sendTapped)))
        await before.skipReceivedActions()
        await before.send(.closed)

        let after = list(kept)
        await after.send(.task)
        await after.receive(\.loaded)

        XCTAssertNil(after.state.openID)
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

    func testSearchingFindsConversationsByNameOrMessage() async throws {
        let milk = chat("Buy milk")
        var trip = chat("Plan the trip")
        trip.name = "Milk run"
        let other = chat("Call mum")
        let store = list(.inMemory([milk, trip, other]))
        await store.send(.task)
        await store.skipReceivedActions()

        await store.send(.searched("milk"))
        XCTAssertEqual(Set(store.state.found.map(\.id)), [milk.id, trip.id])
        XCTAssertNil(store.state.found.first { $0.id == trip.id }?.message, "Its name has it")
        XCTAssertNotNil(store.state.found.first { $0.id == milk.id }?.message)

        await store.send(.searched(""))
        XCTAssertTrue(store.state.found.isEmpty)
    }

    func testOpeningAMessageFoundOnAnotherBranchShowsThatBranch() async throws {
        var trip = chat("Plan the trip")
        let hotel = ChatMessage(id: UUID(), role: .person, text: "Book a hotel", at: now.addingTimeInterval(1))
        trip.append(hotel)
        let first = try XCTUnwrap(trip.messages.first)
        trip.go(to: first.id)
        trip.append(ChatMessage(id: UUID(), role: .person, text: "Book a flight", at: now.addingTimeInterval(2)))
        let kept = ConversationsClient.inMemory([trip])
        let store = list(kept)
        await store.send(.task)
        await store.skipReceivedActions()

        await store.send(.searched("hotel"))
        let found = try XCTUnwrap(store.state.found.first)
        await store.send(.foundTapped(found.id, messageID: found.message?.id))
        await store.finish()

        XCTAssertEqual(store.state.openID, trip.id)
        XCTAssertEqual(store.state.chat?.conversation.messages.map(\.text), ["Plan the trip", "Book a hotel"])
        let saved = try await kept.load()
        XCTAssertEqual(saved.first?.messages.last?.id, hotel.id)
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
        let attachments = AttachmentsClient.inMemory()
        let photo = try await attachments.save(Data([1]), .image, nil, second.id)
        let store = list(kept, attachments: attachments)
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
        XCTAssertNil(attachments.file(photo, second.id))
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

    func testTheRenameSheetSaysWhichModelSuggestsTheName() async {
        let library = InMemoryModelLibrary(installed: [ModelCatalog.gemma4E2B])
        let store = TestStore(initialState: RenameFeature.State(chat("Plan the week"))) {
            RenameFeature()
        } withDependencies: {
            $0.models = library.client
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.suggesterFound)
        XCTAssertEqual(store.state.suggester?.name, ModelCatalog.apple.name)
        XCTAssertEqual(store.state.suggester?.isApple, true)

        await library.client.select(ModelCatalog.gemma4E2B.id, .assistant)
        await store.send(.task)
        await store.receive(\.suggesterFound)
        XCTAssertEqual(store.state.suggester?.name, ModelCatalog.gemma4E2B.name)
        XCTAssertEqual(store.state.suggester?.isApple, false)
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
