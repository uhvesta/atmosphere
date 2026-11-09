import CaptureKit
import ChatFeatures
import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrConversations
import LifrCore
import LifrModelsClient
import LifrSearchClient
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
        dictation: DictationClient = .scripted(), photos: PhotoLibraryClient = .denied,
        files: [String: PickedFile] = [:],
        answer: @escaping @Sendable (AssistantRequest) async throws -> AssistantReply
    ) -> TestStoreOf<ChatFeature> {
        // The shared chat takes photos only for a model its library says
        // reads them.
        let seeing = ModelCatalog.gemma4E4B
        var conversation = Conversation(id: conversationID, createdAt: now)
        if inputs.contains(.image) { conversation.context.model = ModelChoice(model: seeing.id) }
        let store = TestStore(
            initialState: ChatFeature.State(conversation: conversation, workspace: workspace)
        ) {
            ChatFeature()
        } withDependencies: { [now] in
            $0.store = .inMemory(workspace, actor: .user(device: "iPhone", surface: .iPhone))
            $0.models = InMemoryModelLibrary(installed: inputs.contains(.image) ? [seeing] : []).client
            $0.skills = .inMemory()
            $0.guidance = .inMemory()
            $0.assistant.availability = { _ in .available }
            $0.assistant.pickSkill = { _, _, _, _ in nil }
            $0.assistant.prewarm = { _ in prewarmed.withValue { $0 += 1 } }
            $0.assistant.inputs = { _ in inputs }
            $0.attachments = attachments
            $0.recorder = recorder
            $0.dictation = dictation
            $0.photoLibrary = photos
            $0.fileReader = .scripted(files)
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

    /// A reply proposing `changes`, as the live assistant writes one.
    private static func proposing(
        _ changes: [TaskChange], in request: AssistantRequest, text: String = "OK."
    ) async -> AssistantReply {
        let tools = request.tools()
        for change in changes { _ = try? await tools.change(change) }
        var reply = AssistantReply(text: text, isComplete: true)
        reply.proposal = tools.proposal
        return reply
    }

    /// Applies what the last reply proposed.
    private func applyLast(in store: TestStoreOf<ChatFeature>) async throws {
        let reply = try XCTUnwrap(store.state.conversation.messages.last)
        await store.send(.applyTapped(messageID: reply.id))
        await store.skipReceivedActions(strict: false)
    }

    private func send(_ text: String, in store: TestStoreOf<ChatFeature>) async {
        await store.send(.core(.task))
        await store.send(.core(.binding(.set(\.input, text))))
        await store.send(.core(.sendTapped))
        await store.skipReceivedActions(strict: false)
    }

    func testAChatsModelThinkingAndWindowGoWithEachMessageAndChangeAfterTheFact() async throws {
        let (workspace, _) = home()
        let asked = LockIsolated<[ModelChoice]>([])
        let library = InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small])
        let store = chat(workspace) { request in
            asked.withValue { $0.append(request.model) }
            return AssistantReply(text: "OK.", isComplete: true)
        }
        store.dependencies.models = library.client
        let qwen = ModelCatalog.qwen35Small.id

        // Picked before the first message.
        await store.send(.core(.modelTapped))
        await store.send(.model(.presented(.task)))
        await store.receive(\.model.presented.loaded)
        XCTAssertEqual(store.state.model?.models.map(\.id), [.apple, qwen])
        await store.send(.model(.presented(.binding(.set(\.model, qwen)))))
        await store.send(.model(.presented(.binding(.set(\.choice.thinking, .long)))))
        await store.send(.model(.presented(.binding(.set(\.choice.window, 16_384)))))
        await store.send(.model(.dismiss))
        await send("Plan the week", in: store)

        // Changed after it, for the next message.
        await store.send(.core(.modelTapped))
        await store.send(.model(.presented(.binding(.set(\.choice.thinking, .off)))))
        await store.send(.model(.dismiss))
        await send("And the weekend", in: store)

        XCTAssertEqual(
            asked.value,
            [
                ModelChoice(model: qwen, thinking: .long, window: 16_384),
                ModelChoice(model: qwen, thinking: .off, window: 16_384),
            ])
        XCTAssertEqual(store.state.conversation.context.model.thinking, .off)
        XCTAssertEqual(library.client.selection(.assistant), .apple)
    }

    func testAForkRunsOnTheSameModelAsItsChat() throws {
        var conversation = Conversation(id: conversationID, createdAt: now)
        conversation.context.model = ModelChoice(model: ModelCatalog.qwen35Small.id, thinking: .off)
        conversation.context.picksSkills = false
        let message = ChatMessage(id: UUID(1), role: .person, text: "Plan the week", at: now)
        conversation.messages = [message]

        let fork = try XCTUnwrap(conversation.fork(at: message.id, as: UUID(2), now: now))

        XCTAssertEqual(fork.context.model, conversation.context.model)
        XCTAssertEqual(fork.context.picksSkills, false)
    }

    func testAPhotoAndARecordingGoWithTheNextMessageToTheModel() async throws {
        let (workspace, _) = home()
        let asked = LockIsolated<AssistantRequest?>(nil)
        let store = chat(workspace, inputs: [.image, .audio]) { request in
            asked.setValue(request)
            return AssistantReply(text: "A fern.", isComplete: true)
        }
        await store.send(.core(.task))

        await store.send(.core(.photoPicked(Data([1]))))
        await store.skipReceivedActions(strict: false)
        await store.send(.core(.micTapped))
        await store.skipReceivedActions(strict: false)
        XCTAssertTrue(store.state.recording)
        XCTAssertFalse(store.state.canSend)
        await store.send(.core(.micTapped))
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(store.state.core.pendingPhotos.count, 1)
        XCTAssertEqual(store.state.recordings.map(\.seconds), [4])
        XCTAssertTrue(store.state.canSend)

        await store.send(.core(.sendTapped))
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(store.state.core.pendingPhotos, [])
        XCTAssertEqual(store.state.recordings, [])
        let sent = try XCTUnwrap(store.state.conversation.messages.first)
        XCTAssertEqual(sent.attachments.map(\.kind), [.image, .audio])
        XCTAssertEqual(asked.value?.attachments.map(\.kind), [.image, .audio])
        XCTAssertEqual(store.state.conversation.messages.last?.text, "A fern.")
    }

    func testTheMicRecordsForAModelThatListensAndDictatesForOneThatDoesnt() async {
        let (workspace, _) = home()
        let listens = chat(workspace, inputs: [.audio]) { _ in AssistantReply(text: "", isComplete: true) }
        await listens.send(.core(.task))
        await listens.send(.core(.micTapped))
        XCTAssertTrue(listens.state.recording)
        XCTAssertFalse(listens.state.core.dictating)
        await listens.send(.cancelRecordingTapped)

        let reads = chat(workspace, inputs: [.image]) { _ in AssistantReply(text: "", isComplete: true) }
        await reads.send(.core(.task))
        await reads.send(.core(.micTapped))
        XCTAssertTrue(reads.state.core.dictating)
        XCTAssertFalse(reads.state.recording)
        await reads.send(.core(.micTapped))
        await reads.receive(\.core.dictationEnded)
    }

    func testWithoutTheMicrophoneNothingIsRecorded() async {
        let (workspace, _) = home()
        var recorder = RecorderClient.scripted()
        recorder.start = { throw AttachmentError.microphoneDenied }
        let store = chat(workspace, inputs: [.audio], recorder: recorder) { _ in
            AssistantReply(text: "", isComplete: true)
        }
        await store.send(.core(.task))

        await store.send(.core(.micTapped))
        await store.skipReceivedActions(strict: false)

        XCTAssertFalse(store.state.recording)
        XCTAssertNotNil(store.state.alert)
    }

    func testACancelledRecordingAttachesNothing() async {
        let (workspace, _) = home()
        let store = chat(workspace, inputs: [.audio]) { _ in AssistantReply(text: "", isComplete: true) }
        await store.send(.core(.task))

        await store.send(.core(.micTapped))
        await store.skipReceivedActions(strict: false)
        await store.send(.cancelRecordingTapped)
        await store.skipReceivedActions(strict: false)

        XCTAssertFalse(store.state.recording)
        XCTAssertEqual(store.state.recordings, [])
    }

    func testAttachmentsSentFromAnotherDeviceAreMarked() async {
        let (workspace, _) = home()
        var conversation = Conversation(id: conversationID, createdAt: now)
        let recording = ChatAttachment(id: UUID(9), kind: .audio, seconds: 3)
        conversation.messages = [
            ChatMessage(id: UUID(8), role: .person, text: "", at: now, attachments: [recording])
        ]
        let store = chat(workspace) { _ in AssistantReply(text: "", isComplete: true) }
        await store.send(.binding(.set(\.conversation, conversation)))

        await store.send(.core(.task))

        XCTAssertEqual(store.state.elsewhere, [recording.id])
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
        await store.send(.core(.task))

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
        await store.send(.core(.task))
        await store.send(.editDraftTapped)
        XCTAssertFalse(store.state.editsDraft)

        await send("Plan repotting", in: store)
        await store.send(.editDraftTapped)
        XCTAssertTrue(store.state.editsDraft)

        await store.send(.addTapped)
        await store.skipReceivedActions()
        XCTAssertFalse(store.state.editsDraft)
    }

    func testAProposalIsSavedOnlyOnceAppliedAndUndoPutsItBack() async throws {
        let (workspace, rent) = home()
        let code = workspace.displayCode(of: rent)
        let store = chat(workspace) { request in
            await Self.proposing([TaskChange(code: code, status: "done")], in: request, text: "Marked it done.")
        }

        await send("I paid the rent", in: store)
        let reply = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertEqual(store.state.workspace.tasks[rent.id]?.status, .todo, "nothing changes until it's applied")
        XCTAssertEqual(reply.proposal?.steps.map(\.record), [RecordID(.task, rent.id)])
        XCTAssertNil(reply.proposal?.decision)
        let untouched = try await store.dependencies.store.load()
        XCTAssertEqual(untouched.journal, [])

        try await applyLast(in: store)
        XCTAssertEqual(store.state.workspace.tasks[rent.id]?.status, .done)
        XCTAssertEqual(store.state.proposal(of: reply.id)?.decision, .applied)
        XCTAssertEqual(store.state.workspace.journal.map(\.actor), [.assistant(device: "iPhone")])
        XCTAssertEqual(store.state.workspace.journal.map(\.conversation), [conversationID])
        XCTAssertNotNil(store.state.conversation.changes[reply.id])
        await store.send(.applyTapped(messageID: reply.id))
        XCTAssertEqual(store.state.workspace.journal.count, 1, "an applied proposal isn't applied again")

        await store.send(.undoTapped(messageID: reply.id))
        await store.skipReceivedActions()
        XCTAssertEqual(store.state.workspace.tasks[rent.id]?.status, .todo)
        XCTAssertNil(store.state.conversation.changes[reply.id])
    }

    func testDecliningAProposalSavesNothing() async throws {
        let (workspace, rent) = home()
        let code = workspace.displayCode(of: rent)
        let store = chat(workspace) { request in
            await Self.proposing([TaskChange(code: code, status: "done")], in: request)
        }

        await send("I paid the rent", in: store)
        let reply = try XCTUnwrap(store.state.conversation.messages.last)
        await store.send(.declineTapped(messageID: reply.id))
        await store.send(.applyTapped(messageID: reply.id))
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(store.state.proposal(of: reply.id)?.decision, .declined)
        let saved = try await store.dependencies.store.load()
        XCTAssertEqual(saved.journal, [])
        XCTAssertEqual(saved.tasks[rent.id]?.status, .todo)
    }

    func testAnEditedProposalIsCheckedAgainAndAppliesAsEdited() async throws {
        let (workspace, rent) = home()
        let code = workspace.displayCode(of: rent)
        let store = chat(workspace) { request in
            await Self.proposing([TaskChange(code: code, status: "done")], in: request)
        }
        await send("I paid the rent", in: store)
        let reply = try XCTUnwrap(store.state.conversation.messages.last)

        await store.send(.editProposalTapped(messageID: reply.id))
        var edit = try XCTUnwrap(store.state.editingProposal)
        edit.actions = [.task(TaskChange(code: code, status: "lots"))]
        await store.send(.binding(.set(\.editingProposal, edit)))
        await store.send(.editProposalDone)
        await store.skipReceivedActions(strict: false)
        XCTAssertNotNil(store.state.editingProposal?.problem, "a status Lifr doesn't know is said in the editor")

        edit.actions = [.task(TaskChange(code: code, status: "in_progress"))]
        await store.send(.binding(.set(\.editingProposal, edit)))
        await store.send(.editProposalDone)
        await store.skipReceivedActions(strict: false)
        XCTAssertNil(store.state.editingProposal)
        try await applyLast(in: store)

        XCTAssertEqual(store.state.workspace.tasks[rent.id]?.status, .inProgress)
    }

    func testLeavingOutEveryChangeDeclinesTheProposal() async throws {
        let (workspace, rent) = home()
        let code = workspace.displayCode(of: rent)
        let store = chat(workspace) { request in
            await Self.proposing([TaskChange(code: code, status: "done")], in: request)
        }
        await send("I paid the rent", in: store)
        let reply = try XCTUnwrap(store.state.conversation.messages.last)

        await store.send(.editProposalTapped(messageID: reply.id))
        var edit = try XCTUnwrap(store.state.editingProposal)
        edit.kept = [false]
        await store.send(.binding(.set(\.editingProposal, edit)))
        await store.send(.editProposalDone)

        XCTAssertNil(store.state.editingProposal)
        XCTAssertEqual(store.state.proposal(of: reply.id)?.decision, .declined)
    }

    func testAProposalForATaskTrashedSinceSaysSoAndChangesNothing() async throws {
        let (workspace, rent) = home()
        let code = workspace.displayCode(of: rent)
        let store = chat(workspace) { request in
            await Self.proposing([TaskChange(code: code, status: "done")], in: request)
        }
        await send("I paid the rent", in: store)
        _ = try await withDependencies {
            $0.date.now = now
            $0.uuid = .incrementing
        } operation: {
            try await store.dependencies.store.moveToTrash(rent.id)
        }

        try await applyLast(in: store)

        XCTAssertNotNil(store.state.alert)
        XCTAssertNil(store.state.proposal(of: try XCTUnwrap(store.state.conversation.messages.last).id)?.decision)
        let saved = try await store.dependencies.store.load()
        XCTAssertEqual(saved.journal.count, 1, "only the trashing")
    }

    func testAReplysChangesUndoneElsewhereNoLongerOfferUndo() async throws {
        let (workspace, rent) = home()
        let context = Context(actor: .user(device: "Mac", surface: .mac), now: now)
        var changed = workspace
        let done = try changed.perform(in: context) { workspace, context throws(LifrError) in
            try workspace.setStatus(.done, of: rent.id, in: context)
        }
        let entry = try XCTUnwrap(done.entry)
        let reply = ChatMessage(id: UUID(9), role: .assistant, text: "Marked it done.", at: now)
        var conversation = Conversation(id: conversationID, createdAt: now)
        conversation.messages = [reply]
        conversation.changes[reply.id] = [entry]
        var state = ChatFeature.State(conversation: conversation, workspace: changed)

        var undone = changed
        undone.perform(in: context) { workspace, context in workspace.undo(done, in: context) }
        state.show(Snapshot(undone, number: 1))
        XCTAssertNil(state.conversation.changes[reply.id])
        XCTAssertEqual(state.workspace.tasks[rent.id]?.status, .todo)

        let reopened = ChatFeature.State(conversation: conversation, workspace: undone)
        XCTAssertNil(reopened.conversation.changes[reply.id])
    }

    func testAReplyWithoutThoughtsShowsNoThinking() async throws {
        let (workspace, _) = home()
        let empty = ModelThinking(text: "  ", duration: .seconds(1), processor: .gpu)
        let store = chat(workspace) { _ in AssistantReply(text: "Paid.", isComplete: true, thinking: empty) }

        await send("I paid the rent", in: store)

        XCTAssertNil(store.state.conversation.messages.last?.thinking)
    }

    func testAReplyKeepsHowFastItWentAndShowsWhatTheModelWasSent() async throws {
        let (workspace, _) = home()
        let speed = TurnSpeed(read: 900, written: 30, reading: .seconds(1), writing: .seconds(2))
        let store = chat(workspace) { _ in
            var reply = AssistantReply(text: "Paid.", isComplete: true)
            reply.speed = speed
            reply.sent = AssistantSent(instructions: "You help with Lifr.", prompt: "I paid the rent", photos: 0)
            return reply
        }

        await send("I paid the rent", in: store)
        let reply = try XCTUnwrap(store.state.conversation.messages.last)
        await store.send(.core(.promptTapped(messageID: reply.id)))

        XCTAssertEqual(reply.speed?.read, 900)
        XCTAssertEqual(reply.speed?.written, 30)
        XCTAssertEqual(store.state.core.shownPromptValue?.instructions, "You help with Lifr.")
        XCTAssertEqual(store.state.core.shownPromptValue?.prompt, "I paid the rent")
    }

    func testAFailedReplyKeepsTheMessageAndSaysWhy() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in throw AssistantError.tooLong }

        await send("Hello", in: store)
        XCTAssertNil(store.state.core.reply)
        XCTAssertNotNil(store.state.core.problem)
        XCTAssertEqual(store.state.conversation.messages.map(\.text), ["Hello"])
        let hello = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertFalse(store.state.core.canRetry(hello), "Sending a message that's too long again can't help")
    }

    func testRetrySendsAnUnreadableReplysMessageAgain() async throws {
        let (workspace, _) = home()
        let attempts = LockIsolated(0)
        let store = chat(workspace) { _ in
            let attempt = attempts.withValue {
                $0 += 1
                return $0
            }
            if attempt == 1 { throw AssistantError.unreadable }
            return AssistantReply(text: "Done.", isComplete: true)
        }

        await send("Hello", in: store)
        let hello = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertTrue(store.state.core.canRetry(hello))

        await store.send(.core(.retryTapped(messageID: hello.id)))
        await store.skipReceivedActions(strict: false)
        XCTAssertNil(store.state.core.problem)
        XCTAssertEqual(store.state.conversation.messages.map(\.text), ["Hello", "Done."])
    }

    func testOpeningTheChatLoadsNoModelUntilThePersonWrites() async {
        let (workspace, _) = home()
        let prewarmed = LockIsolated(0)
        let store = chat(workspace, prewarmed: prewarmed) { _ in AssistantReply(text: "", isComplete: true) }
        store.exhaustivity = .off

        await store.send(.core(.task))
        XCTAssertEqual(prewarmed.value, 0)

        await store.send(.core(.binding(.set(\.input, "A"))))
        await store.send(.core(.binding(.set(\.input, "Ad"))))
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

        await clock.advance(by: LifrReplier.stallLimit * 2)
        await store.receive(\.core.replied.failure)
        XCTAssertNil(store.state.core.reply)
        let hello = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertNotNil(store.state.core.problem)
        XCTAssertTrue(store.state.core.canRetry(hello))
    }

    func testAReplyThatStallsAfterProposingCanBeTriedAgain() async throws {
        let (workspace, rent) = home()
        let clock = TestClock()
        let code = workspace.displayCode(of: rent)
        let store = chat(workspace, clock: clock) { request in
            _ = try? await request.tools().change(TaskChange(code: code, status: "done"))
            try await Task.sleep(for: .seconds(3_600))
            return AssistantReply(text: "Late.", isComplete: true)
        }

        await send("Finish rent", in: store)
        await clock.advance(by: LifrReplier.stallLimit * 2)
        await store.receive(\.core.replied.failure)
        let finish = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertNotNil(store.state.core.problem)
        XCTAssertTrue(store.state.core.canRetry(finish), "nothing was saved, so sending it again is safe")
    }

    func testAReplyThatKeepsWritingDoesntTimeOut() async throws {
        let (workspace, _) = home()
        let clock = TestClock()
        let store = chat(workspace, clock: clock) { _ in AssistantReply(text: "", isComplete: false) }
        store.dependencies.assistant.reply = { _ in
            AssistantReplies { continuation in
                Task {
                    for step in 1...3 {
                        try await clock.sleep(for: LifrReplier.stallLimit / 2)
                        continuation.yield(AssistantReply(text: "Part \(step)"))
                    }
                    continuation.yield(AssistantReply(text: "Done.", isComplete: true))
                    continuation.finish()
                }
            }
        }

        await send("Hello", in: store)
        for _ in 1...3 { await clock.advance(by: LifrReplier.stallLimit / 2) }
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
        await clock.advance(by: LifrReplier.stallLimit)
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

    func testStoppingAReplyKeepsWhatItWroteAndLetsThePersonSendAgain() async throws {
        let (workspace, _) = home()
        let finish = AsyncStream.makeStream(of: Void.self)
        let store = chat(workspace) { _ in AssistantReply(text: "", isComplete: false) }
        store.dependencies.assistant.reply = { _ in
            AssistantReplies { continuation in
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
        for _ in 0..<100 where store.state.core.reply != "Half" {
            try await Task.sleep(for: .milliseconds(10))
            await store.skipReceivedActions(strict: false)
        }
        XCTAssertEqual(store.state.core.reply, "Half")
        XCTAssertTrue(store.state.canStop)

        await store.send(.core(.stopTapped))
        finish.continuation.yield()
        await store.skipReceivedActions(strict: false)

        XCTAssertFalse(store.state.canStop)
        XCTAssertNil(store.state.core.reply)
        XCTAssertEqual(store.state.conversation.messages.map(\.text), ["Hello", "Half"])
        await store.send(.core(.binding(.set(\.input, "Again"))))
        XCTAssertTrue(store.state.canSend)
    }

    func testStoppingBeforeAnyWordsAddsNoReply() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in
            try await Task.sleep(for: .seconds(60))
            return AssistantReply(text: "Too late.", isComplete: true)
        }

        await send("Hello", in: store)
        await store.send(.core(.stopTapped))
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(store.state.conversation.messages.map(\.text), ["Hello"])
        XCTAssertNil(store.state.alert)
        XCTAssertNil(store.state.core.problem)
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
        XCTAssertNotNil(store.state.core.reply)
        XCTAssertNotNil(store.state.core.reply)
        XCTAssertNil(store.state.alert)
        XCTAssertFalse(store.state.canSend)

        await store.send(.resume)
        await store.skipReceivedActions()
        XCTAssertNil(store.state.core.reply)
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
        await store.send(.core(.binding(.set(\.input, "Hello again"))))
        await store.send(.core(.sendTapped))
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

        await store.send(.core(.task))
        await store.skipReceivedActions()
        await store.send(.core(.binding(.set(\.input, "/ti"))))
        XCTAssertEqual(store.state.slashChoices.first?.id, "tidy")
        await store.send(.pickSkillTapped("tidy"))
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
        await store.send(.core(.binding(.set(\.input, "/"))))
        let skills = store.state.slashChoices.compactMap { if case .skill(let skill) = $0 { skill.id } else { nil } }
        XCTAssertEqual(skills, ["tidy"])
        await store.send(.pickSkillTapped("query"))
        XCTAssertNil(store.state.picked)
        await store.send(.core(.binding(.set(\.input, "/query "))))
        XCTAssertNil(store.state.picked)
        XCTAssertEqual(store.state.input, "/query ")
    }

    // MARK: - Typing a skill's name

    func testASkillsNameThenASpacePicksIt() async {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "", isComplete: true) }

        await store.send(.core(.binding(.set(\.input, "/add"))))
        XCTAssertNil(store.state.picked, "it may still be the start of a longer name")
        XCTAssertEqual(store.state.input, "/add")
        await store.send(.core(.binding(.set(\.input, "/add "))))

        XCTAssertEqual(store.state.picked?.id, "add")
        XCTAssertEqual(store.state.input, "")
        XCTAssertTrue(store.state.slashChoices.isEmpty)
    }

    func testANameTypedOnPastKeepsWhatFollowsAsTheDetails() async {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "", isComplete: true) }

        await store.send(.core(.binding(.set(\.input, "/Add buy milk"))))
        XCTAssertEqual(store.state.picked?.id, "add")
        XCTAssertEqual(store.state.input, "buy milk")

        await store.send(.clearSkillTapped)
        await store.send(.core(.binding(.set(\.input, "/tidy\nOnly HOME"))))
        XCTAssertEqual(store.state.picked?.id, "tidy")
        XCTAssertEqual(store.state.input, "Only HOME")
    }

    func testTheStartOfALongerNameDoesntPickTheShorterOne() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "", isComplete: true) }
        let addresses = try XCTUnwrap(
            Skill(
                userNamed: "addresses", summary: "Find addresses.", instructions: "Find addresses.", tools: [],
                draftsPlans: false))
        await store.send(.skillsLoaded(Skill.builtIn + [addresses]))

        for typed in ["/add", "/addr", "/addresses"] {
            await store.send(.core(.binding(.set(\.input, typed))))
            XCTAssertNil(store.state.picked, typed)
        }
        await store.send(.core(.binding(.set(\.input, "/addresses near home"))))
        XCTAssertEqual(store.state.picked?.id, addresses.id)
        XCTAssertEqual(store.state.input, "near home")
    }

    func testANameNoSkillHasOrOneMidwayStaysText() async {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "", isComplete: true) }

        for typed in ["/groceries ", "buy /add milk", "/ add"] {
            await store.send(.core(.binding(.set(\.input, typed))))
            XCTAssertNil(store.state.picked, typed)
            XCTAssertEqual(store.state.input, typed)
        }
    }

    func testSendingASkillsNameAndDetailsSendsThemWithTheSkill() async {
        let (workspace, _) = home()
        let sent = LockIsolated<[Skill?]>([])
        let store = chat(workspace) { request in
            sent.withValue { $0.append(request.skill) }
            return AssistantReply(text: "Added.", isComplete: true)
        }

        await send("/add buy milk", in: store)
        XCTAssertEqual(store.state.conversation.messages.first?.text, "buy milk")
        XCTAssertEqual(store.state.conversation.messages.first?.skill, "add")

        // Sent with nothing after the name, it's still the skill's.
        await send("/review", in: store)
        XCTAssertEqual(store.state.conversation.messages.dropFirst(2).first?.text, "")
        XCTAssertEqual(store.state.conversation.messages.dropFirst(2).first?.skill, "review")
        XCTAssertEqual(sent.value.map { $0?.id }, ["add", "review"])
    }

    func testWithASkillPickedASlashStaysText() async {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "", isComplete: true) }
        await store.send(.pickSkillTapped("review"))

        await store.send(.core(.binding(.set(\.input, "/add more"))))
        XCTAssertEqual(store.state.picked?.id, "review")
        XCTAssertEqual(store.state.input, "/add more")
    }

    func testRemovingThePickedSkillKeepsTheDetails() async {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "", isComplete: true) }
        await store.send(.core(.binding(.set(\.input, "/add buy milk"))))

        await store.send(.clearSkillTapped)
        XCTAssertNil(store.state.picked)
        XCTAssertEqual(store.state.input, "buy milk")
    }

    // MARK: - Commands

    func testTheSlashListOffersOnlyCommandsThatCanRunNow() async {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "OK.", isComplete: true) }
        func commands() -> [ChatCommand] {
            store.state.slashChoices.compactMap { if case .command(let command) = $0 { command } else { nil } }
        }

        await store.send(.core(.binding(.set(\.input, "/"))))
        XCTAssertEqual(commands(), [.rename, .model, .skills, .new])
        await send("Plan the week", in: store)
        await store.send(.core(.binding(.set(\.input, "/"))))
        XCTAssertTrue(commands().contains(.find))
        XCTAssertFalse(commands().contains(.forward))
        await store.send(.core(.binding(.set(\.input, "/fi"))))
        XCTAssertEqual(commands(), [.find])

        // Names starting with what's typed come before looser matches.
        await store.send(.core(.binding(.set(\.input, "/c"))))
        XCTAssertEqual(store.state.slashChoices.prefix(2).map(\.id), ["change", "checkpoints"])
    }

    func testACommandNotAvailableStaysText() async {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "", isComplete: true) }

        await store.send(.core(.binding(.set(\.input, "/back "))))
        XCTAssertEqual(store.state.input, "/back ")
    }

    func testFindTypedWithWordsFindsThem() async {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "Saturday works.", isComplete: true) }
        await send("Plan saturday", in: store)

        await store.send(.core(.binding(.set(\.input, "/find saturday"))))
        await store.send(.core(.sendTapped))
        XCTAssertTrue(store.state.core.finds)
        XCTAssertEqual(store.state.core.findQuery, "saturday")
        XCTAssertEqual(store.state.core.findMatches.count, 2)
        XCTAssertEqual(store.state.input, "")
        XCTAssertEqual(store.state.conversation.messages.count, 2, "nothing was sent")
    }

    func testACommandRunsOnceTypedWithASpaceOrPicked() async {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "OK.", isComplete: true) }
        await send("Plan the week", in: store)

        await store.send(.core(.binding(.set(\.input, "/checkpoints "))))
        XCTAssertTrue(store.state.showsCheckpoints)
        XCTAssertEqual(store.state.input, "")

        await store.send(.core(.binding(.set(\.input, "/mod"))))
        await store.send(.pickCommandTapped(.model))
        await store.receive(\.core.modelTapped)
        XCTAssertNotNil(store.state.model)
        XCTAssertEqual(store.state.input, "")

        await store.send(.core(.binding(.set(\.input, "/rename "))))
        await store.receive(\.delegate.rename)
    }

    func testBackTypedGoesBackAndForwardReturns() async {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "OK.", isComplete: true) }
        ticking(store)
        await send("Plan the week", in: store)
        await send("And the weekend", in: store)

        await store.send(.core(.binding(.set(\.input, "/back "))))
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(store.state.conversation.messages.count, 2)
        await store.send(.core(.binding(.set(\.input, "/forward"))))
        await store.send(.core(.sendTapped))
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(store.state.conversation.messages.count, 4)
    }

    func testNewTypedInAChatOpensANewOne() async throws {
        let store = list(.inMemory())
        await store.send(.task)
        await store.send(.newTapped)
        let id = try XCTUnwrap(store.state.openID)
        await store.send(.chats(.element(id: id, action: .core(.binding(.set(\.input, "Plan the week"))))))
        await store.send(.chats(.element(id: id, action: .core(.sendTapped))))
        await store.skipReceivedActions()

        await store.send(.chats(.element(id: id, action: .core(.binding(.set(\.input, "/new "))))))
        await store.skipReceivedActions()
        XCTAssertNotEqual(store.state.openID, id)
        XCTAssertEqual(store.state.chat?.conversation.messages.isEmpty, true)
    }

    func testTypingASlashListsTheSkillsThatMatchAndPickingOneTakesItsPlace() async {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "", isComplete: true) }
        await store.send(.core(.binding(.set(\.input, "ti"))))
        XCTAssertTrue(store.state.slashChoices.isEmpty)

        await store.send(.core(.binding(.set(\.input, "/ti"))))
        XCTAssertEqual(store.state.slashChoices.first?.id, "tidy")
        await store.send(.pickSkillTapped("tidy"))

        XCTAssertEqual(store.state.picked?.id, "tidy")
        XCTAssertEqual(store.state.input, "")
        await store.send(.core(.binding(.set(\.input, "/ti"))))
        XCTAssertTrue(store.state.slashChoices.isEmpty)
    }

    func testTheComposerNamesTheModelTheChatRunsOn() async {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "", isComplete: true) }
        store.dependencies.models = InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small]).client
        XCTAssertNil(store.state.core.current)

        await store.send(.core(.task))
        await store.receive(\.core.modelsLoaded)
        XCTAssertEqual(store.state.core.current?.id, .apple)
        await store.send(.core(.modelTapped))
        await store.send(.model(.presented(.binding(.set(\.model, ModelCatalog.qwen35Small.id)))))
        await store.send(.model(.dismiss))

        XCTAssertEqual(store.state.core.current?.id, ModelCatalog.qwen35Small.id)
    }

    func testAReplySaysHowMuchOfTheWindowItTookUntilTheModelChanges() async {
        let (workspace, _) = home()
        let used = ContextUse(used: 1_200, window: 4_096)
        let store = chat(workspace) { _ in
            var reply = AssistantReply(text: "OK.", isComplete: true)
            reply.context = used
            return reply
        }
        store.dependencies.models = InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small]).client
        XCTAssertNil(store.state.core.context)
        await send("Plan the week", in: store)
        XCTAssertEqual(store.state.core.context, used)

        await store.send(.core(.modelTapped))
        await store.send(.model(.presented(.binding(.set(\.choice.thinking, .long)))))
        await store.send(.model(.dismiss))
        XCTAssertEqual(store.state.core.context, used)

        await store.send(.core(.modelTapped))
        await store.send(.model(.presented(.binding(.set(\.model, ModelCatalog.qwen35Small.id)))))
        await store.send(.model(.dismiss))
        XCTAssertNil(store.state.core.context)
    }

    // MARK: - Picking a skill

    func testAMessageSentWithoutASkillGetsTheOneLifrPicksAmongTheAutomaticOnes() async {
        let (workspace, _) = home()
        let offered = LockIsolated<[String]>([])
        let sent = LockIsolated<[Skill?]>([])
        let store = chat(workspace) { request in
            sent.withValue { $0.append(request.skill) }
            return AssistantReply(text: "Which first?", isComplete: true)
        }
        store.dependencies.assistant.pickSkill = { _, skills, _, _ in
            offered.setValue(skills.map(\.id))
            return "breakdown"
        }

        await send("Dentist, taxes, gym, call mum", in: store)

        XCTAssertEqual(Set(offered.value), Set(Skill.builtIn.filter(\.autoPicks).map(\.id)))
        XCTAssertFalse(offered.value.contains("undo"))
        XCTAssertEqual(sent.value, [.breakdown])
        XCTAssertEqual(store.state.conversation.messages.first?.skill, "breakdown", "its pill shows")
    }

    func testWhenLifrPicksNoSkillTheAssistantJustTalks() async {
        let (workspace, _) = home()
        let sent = LockIsolated<[Skill?]>([])
        let store = chat(workspace) { request in
            sent.withValue { $0.append(request.skill) }
            return AssistantReply(text: "Hi!", isComplete: true)
        }

        await send("Hello", in: store)

        XCTAssertEqual(sent.value, [nil])
        XCTAssertEqual(store.state.conversation.messages.first?.skill, nil)
    }

    func testASkillPickedByHandOrPickingTurnedOffAsksNothing() async {
        let (workspace, _) = home()
        let picks = LockIsolated(0)
        let sent = LockIsolated<[Skill?]>([])
        let store = chat(workspace) { request in
            sent.withValue { $0.append(request.skill) }
            return AssistantReply(text: "OK.", isComplete: true)
        }
        store.dependencies.assistant.pickSkill = { _, _, _, _ in
            picks.withValue { $0 += 1 }
            return "query"
        }

        await store.send(.pickSkillTapped("review"))
        await send("How was my week?", in: store)
        store.state.$picksSkills.withLock { $0 = false }
        await send("And today?", in: store)

        XCTAssertEqual(picks.value, 0)
        XCTAssertEqual(sent.value, [.review, nil])
    }

    func testPickingTurnedOffInOneChatPicksOnlyWhatThePersonPicks() async {
        let (workspace, _) = home()
        let picks = LockIsolated(0)
        let sent = LockIsolated<[Skill?]>([])
        let store = chat(workspace) { request in
            sent.withValue { $0.append(request.skill) }
            return AssistantReply(text: "OK.", isComplete: true)
        }
        store.dependencies.assistant.pickSkill = { _, _, _, _ in
            picks.withValue { $0 += 1 }
            return "query"
        }

        await store.send(.core(.modelTapped))
        XCTAssertEqual(store.state.model?.picksSkillsHere, true, "it starts at Settings")
        await store.send(.model(.presented(.binding(.set(\.picksSkillsHere, false)))))
        await store.send(.model(.dismiss))
        await send("How was my week?", in: store)
        await store.send(.pickSkillTapped("review"))
        await send("And today?", in: store)

        XCTAssertEqual(picks.value, 0)
        XCTAssertEqual(sent.value, [nil, .review])
        XCTAssertEqual(store.state.conversation.context.picksSkills, false)
        XCTAssertTrue(store.state.picksSkills, "Settings stays as it is")
    }

    func testPickingTurnedOnInOneChatPicksWhileSettingsDoesnt() async {
        let (workspace, _) = home()
        let sent = LockIsolated<[Skill?]>([])
        let store = chat(workspace) { request in
            sent.withValue { $0.append(request.skill) }
            return AssistantReply(text: "OK.", isComplete: true)
        }
        store.dependencies.assistant.pickSkill = { _, _, _, _ in "query" }
        store.state.$picksSkills.withLock { $0 = false }

        await store.send(.core(.modelTapped))
        await store.send(.model(.presented(.binding(.set(\.picksSkillsHere, true)))))
        await store.send(.model(.dismiss))
        await send("How was my week?", in: store)

        XCTAssertEqual(sent.value, [.query])
    }

    func testWhilePlanningAnAnswerKeepsTheSkillWhenLifrPicksNone() async {
        let (workspace, _) = home()
        let sent = LockIsolated<[Skill?]>([])
        let store = chat(workspace) { request in
            sent.withValue { $0.append(request.skill) }
            return AssistantReply(
                text: "Here's a plan.", plan: Plan(tasks: [.init(title: "Taxes", place: "HOME")]), isComplete: true)
        }
        let picks = LockIsolated<[String?]>(["breakdown", nil])
        let drafting = LockIsolated<[String?]>([])
        store.dependencies.assistant.pickSkill = { _, _, skill, _ in
            drafting.withValue { $0.append(skill) }
            return picks.withValue { $0.removeFirst() }
        }

        await send("Taxes and the dentist", in: store)
        await send("Taxes first", in: store)

        XCTAssertEqual(sent.value, [.breakdown, .breakdown])
        XCTAssertEqual(drafting.value, [nil, "breakdown"], "the picker hears which skill is drafting the plan")
        XCTAssertEqual(store.state.conversation.messages.map(\.skill), ["breakdown", nil, "breakdown", nil])
    }

    func testAFailedPickAnswersWithoutASkill() async {
        let (workspace, _) = home()
        let sent = LockIsolated<[Skill?]>([])
        let store = chat(workspace) { request in
            sent.withValue { $0.append(request.skill) }
            return AssistantReply(text: "Hi!", isComplete: true)
        }
        store.dependencies.assistant.pickSkill = { _, _, _, _ in throw AssistantError.busy }

        await send("Hello", in: store)

        XCTAssertEqual(sent.value, [nil])
        XCTAssertEqual(store.state.conversation.messages.last?.text, "Hi!")
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

        await store.send(.core(.branchTapped(messageID: again.id, by: -1)))
        XCTAssertEqual(texts(store), ["one", "re one", "two", "re two"])
        let two = try XCTUnwrap(store.state.conversation.messages.first { $0.text == "two" })
        await store.send(.core(.branchTapped(messageID: two.id, by: 1)))
        XCTAssertEqual(texts(store), ["one", "re one", "two again", "re two again"])
    }

    func testGoingBackPastChangesAsksWhetherToUndoThem() async throws {
        let (workspace, rent) = home()
        let code = workspace.displayCode(of: rent)
        let store = chat(workspace) { request in
            guard request.messages.last?.text == "I paid the rent" else {
                return AssistantReply(text: "OK.", isComplete: true)
            }
            return await Self.proposing([TaskChange(code: code, status: "done")], in: request)
        }
        ticking(store)
        await send("Hello", in: store)
        await send("I paid the rent", in: store)
        try await applyLast(in: store)
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

        await store.send(.core(.binding(.set(\.input, "three"))))
        await store.send(.core(.sendTapped))
        await store.send(.backTapped)
        XCTAssertEqual(store.state.conversation.messages.last?.text, "three")
    }

    func testAForkFromBeforeTheMessageBeingAnsweredCanBeMadeMeanwhile() async throws {
        let (workspace, _) = home()
        let store = chat(workspace) { _ in AssistantReply(text: "OK.", isComplete: true) }
        ticking(store)
        await send("one", in: store)
        store.dependencies.assistant.reply = { _ in AssistantReplies { _ in } }
        await store.send(.core(.binding(.set(\.input, "two"))))
        await store.send(.core(.sendTapped))
        let messages = store.state.conversation.messages
        let reply = try XCTUnwrap(messages.dropFirst().first)
        let asked = try XCTUnwrap(messages.last)

        XCTAssertFalse(store.state.canFork(at: asked.id), "It has no reply yet to fork with")
        await store.send(.forkTapped(messageID: reply.id))
        await store.receive(\.forked)

        XCTAssertNotNil(store.state.core.reply, "The reply goes on being written")
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

        await store.send(.core(.editTapped(messageID: two.id)))
        XCTAssertEqual(store.state.input, "two")
        await send("two, edited", in: store)

        XCTAssertNil(store.state.core.editing)
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

        await store.send(.core(.editTapped(messageID: one.id)))
        await store.send(.core(.cancelEditTapped))

        XCTAssertNil(store.state.core.editing)
        XCTAssertEqual(store.state.input, "")
        XCTAssertEqual(texts(store), ["one", "OK."])
    }

    func testEditingAMessageBeforeChangesAsksWhetherToUndoThem() async throws {
        let (workspace, rent) = home()
        let code = workspace.displayCode(of: rent)
        let store = chat(workspace) { request in
            guard request.messages.last?.text == "I paid the rent" else {
                return AssistantReply(text: "OK.", isComplete: true)
            }
            return await Self.proposing([TaskChange(code: code, status: "done")], in: request)
        }
        ticking(store)
        await send("I paid the rent", in: store)
        try await applyLast(in: store)
        let paid = try XCTUnwrap(store.state.conversation.messages.first)

        await store.send(.core(.editTapped(messageID: paid.id)))
        await store.send(.core(.binding(.set(\.input, "Hello"))))
        await store.send(.core(.sendTapped))
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
        XCTAssertTrue(store.state.core.canRetry(reply))

        await store.send(.core(.retryTapped(messageID: reply.id)))
        await store.skipReceivedActions(strict: false)

        XCTAssertEqual(texts(store), ["one", "reply 2"])
        let again = try XCTUnwrap(store.state.conversation.messages.last)
        XCTAssertEqual(store.state.conversation.branches(at: again.id).map(\.text), ["reply 1", "reply 2"])
    }

    func testAForkOpensAsANewConversationUpToThere() async throws {
        let kept = ConversationsClient.inMemory()
        let attachments = AttachmentsClient.inMemory()
        // A model that reads photos, so the chat takes one.
        let seeing = InMemoryModelLibrary(installed: [ModelCatalog.gemma4E4B])
        await seeing.client.select(ModelCatalog.gemma4E4B.id, .assistant)
        let store = list(kept, attachments: attachments, models: seeing)
        await store.send(.task)
        await store.send(.newTapped)
        let id = try XCTUnwrap(store.state.openID)
        await store.send(.chats(.element(id: id, action: .core(.task))))
        await store.skipReceivedActions()
        await store.send(.chats(.element(id: id, action: .core(.photoPicked(Data([1]))))))
        await store.skipReceivedActions()
        for text in ["Plan the week", "And the weekend"] {
            await store.send(.chats(.element(id: id, action: .core(.binding(.set(\.input, text))))))
            await store.send(.chats(.element(id: id, action: .core(.sendTapped))))
            await store.skipReceivedActions()
        }
        let original = try XCTUnwrap(store.state.chat?.conversation)
        let reply = try XCTUnwrap(original.messages.dropFirst().first?.id)
        let photo = try XCTUnwrap(original.messages.first?.attachments.first)

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
        _ kept: ConversationsClient, attachments: AttachmentsClient = .inMemory(),
        models: InMemoryModelLibrary = InMemoryModelLibrary()
    ) -> TestStoreOf<ConversationsFeature> {
        let (workspace, _) = home()
        let store = TestStore(initialState: ConversationsFeature.State()) {
            ConversationsFeature()
        } withDependencies: { [now] in
            $0.store = .inMemory(workspace)
            $0.conversations = kept
            $0.models = models.client
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
        await store.send(.chats(.element(id: id, action: .core(.task))))
        await store.send(.chats(.element(id: id, action: .core(.binding(.set(\.input, "Hello"))))))
        await store.send(.chats(.element(id: id, action: .core(.sendTapped))))
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
        await before.send(.chats(.element(id: id, action: .core(.binding(.set(\.input, "Hello"))))))
        await before.send(.chats(.element(id: id, action: .core(.sendTapped))))
        await before.skipReceivedActions()
        await before.send(.chats(.element(id: id, action: .core(.binding(.set(\.input, "And then"))))))

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
        await before.send(.chats(.element(id: id, action: .core(.binding(.set(\.input, "Hello"))))))
        await before.send(.chats(.element(id: id, action: .core(.sendTapped))))
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
        await store.send(.chats(.element(id: id, action: .core(.binding(.set(\.input, "Plan my week"))))))
        await store.send(.chats(.element(id: id, action: .core(.sendTapped))))
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
        await store.send(.chats(.element(id: first, action: .core(.binding(.set(\.input, "First"))))))
        await store.send(.chats(.element(id: first, action: .core(.sendTapped))))
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

    func testRelatedWordsFindConversationsTheWordsTypedDontOnceThePersonPauses() async throws {
        let milk = chat("Buy milk")
        let plumber = chat("Call the plumber")
        let groceries = chat("Groceries for Sunday")
        let clock = TestClock()
        let store = list(.inMemory([milk, plumber, groceries]))
        store.dependencies.relatedWords = .scripted(["milk", "eggs"])
        store.dependencies.continuousClock = clock
        await store.send(.task)
        await store.skipReceivedActions()

        await store.send(.searched("groceries"))
        XCTAssertEqual(store.state.found.map(\.id), [groceries.id])

        await clock.advance(by: .seconds(1))
        await store.receive(\.related)
        XCTAssertEqual(store.state.found.map(\.id), [groceries.id, milk.id], "What has the words typed comes first")
        XCTAssertEqual(store.state.found.map(\.isRelated), [false, true])

        await store.send(.searched("groceries sunday"))
        XCTAssertEqual(store.state.related, [], "A new search finds its own")
    }

    func testRelatedWordsFromAModelWaitToBeAskedFor() async throws {
        let milk = chat("Buy milk")
        let groceries = chat("Groceries for Sunday")
        let clock = TestClock()
        let store = list(.inMemory([milk, groceries]))
        store.dependencies.relatedWords = .scripted(["milk"], from: .model)
        store.dependencies.continuousClock = clock
        await store.send(.task)
        await store.skipReceivedActions()

        await store.send(.searched("groceries"))
        await clock.advance(by: .seconds(1))
        XCTAssertEqual(store.state.found.map(\.id), [groceries.id])
        XCTAssertFalse(store.state.relating)

        await store.send(.relatedTapped)
        await store.receive(\.related)
        XCTAssertEqual(store.state.found.map(\.id), [groceries.id, milk.id])
    }

    func testSearchingForATimeFindsConversationsFromThen() async throws {
        let today = chat("Buy milk")
        var old = Conversation(id: UUID(), createdAt: now.addingTimeInterval(-20 * 86_400))
        old.messages = [ChatMessage(id: UUID(), role: .person, text: "Buy milk", at: old.createdAt)]
        let store = list(.inMemory([today, old]))
        await store.send(.task)
        await store.skipReceivedActions()

        await store.send(.searched("milk today"))

        XCTAssertEqual(store.state.found.map(\.id), [today.id])
        XCTAssertNotNil(store.state.searched.during)
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
            $0.assistant.suggestTitle = { _, _ in "Weekly plan" }
        }
        store.exhaustivity = .off
        await store.send(.suggestTapped)
        await store.receive(\.suggested.success)
        XCTAssertEqual(store.state.name, "Weekly plan")

        store.dependencies.assistant.suggestTitle = { _, _ in throw AssistantError.busy }
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
        await store.receive(\.modelsLoaded)
        XCTAssertEqual(store.state.suggester?.name, ModelCatalog.apple.name)
        XCTAssertEqual(store.state.suggester?.isApple, true)

        await library.client.select(ModelCatalog.gemma4E2B.id, .assistant)
        await store.send(.task)
        await store.receive(\.modelsLoaded)
        XCTAssertEqual(store.state.suggester?.name, ModelCatalog.gemma4E2B.name)
        XCTAssertEqual(store.state.suggester?.isApple, false)
    }

    func testARenameSuggestsWithTheModelPickedForItAndKeepsTheDefault() async {
        let library = InMemoryModelLibrary(installed: [ModelCatalog.gemma4E2B])
        let used = LockIsolated<[ModelID?]>([])
        let store = TestStore(initialState: RenameFeature.State(chat("Plan the week"))) {
            RenameFeature()
        } withDependencies: {
            $0.models = library.client
            $0.assistant.suggestTitle = { _, model in
                used.withValue { $0.append(model) }
                return "Weekly plan"
            }
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.modelsLoaded)
        XCTAssertEqual(store.state.models.map(\.id), [.apple, ModelCatalog.gemma4E2B.id])

        await store.send(.binding(.set(\.model, ModelCatalog.gemma4E2B.id)))
        XCTAssertEqual(store.state.suggester?.name, ModelCatalog.gemma4E2B.name)
        await store.send(.suggestTapped)
        await store.receive(\.suggested.success)

        XCTAssertEqual(used.value, [ModelCatalog.gemma4E2B.id])
        XCTAssertEqual(library.client.selection(.titles), .apple)
    }

    func testTheAssistantNamesANewConversationOnlyWhenTurnedOn() async throws {
        let store = list(.inMemory())
        await store.send(.newTapped)
        let id = try XCTUnwrap(store.state.openID)
        await store.send(.chats(.element(id: id, action: .core(.binding(.set(\.input, "Plan the week"))))))
        await store.send(.chats(.element(id: id, action: .core(.sendTapped))))
        await store.skipReceivedActions()
        await store.finish()
        XCTAssertEqual(store.state.conversations[id: id]?.title, "Plan the week")

        let on = try XCTUnwrap(store.state.chats[id: id]).$namesConversations
        on.withLock { $0 = true }
        await store.send(.newTapped)
        let named = try XCTUnwrap(store.state.openID)
        await store.send(.chats(.element(id: named, action: .core(.binding(.set(\.input, "Plan the week"))))))
        await store.send(.chats(.element(id: named, action: .core(.sendTapped))))
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

        await store.send(.chats(.element(id: id, action: .core(.binding(.set(\.input, "Plan the week"))))))
        await store.send(.chats(.element(id: id, action: .core(.sendTapped))))
        await store.skipReceivedActions()
        await store.finish()
        XCTAssertEqual(store.state.conversations[id: id]?.title, "Mine")
    }
}
