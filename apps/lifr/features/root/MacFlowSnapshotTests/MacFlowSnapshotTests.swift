import ComposableArchitecture
import FlowKit
import LifrAPI
import LifrAssistantClient
import LifrAssistantLive
import LifrBugReport
import LifrConversations
import LifrCore
import LifrGlance
import LifrGlanceClient
import LifrLockClient
import LifrModelsClient
import LifrRemindersClient
import LifrRoot
import LifrSessionClient
import LifrShortcuts
import LifrStoreClient
import LifrSyncClient
import LifrWidgetViews
import SwiftUI
import XCTest

/// Each of a person's flows on the Mac, through the live app on the sample
/// records as of 2026-11-03 10:00 UTC: one reference per step, the
/// accessibility tree and an image of every window, so the screens can be
/// read in order.
@MainActor
final class MacFlowSnapshotTests: XCTestCase {
    override func setUp() async throws {
        try FlowRecorder.checkScreen()
    }

    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    /// The time the app thinks it is, which a flow can move on.
    private let time = LockIsolated(Date(timeIntervalSince1970: 1_793_700_000))
    /// The app's records, for another writer such as Shortcuts.
    private var records: InMemorySync?
    /// What the app last gave the widgets.
    private let widgets = WidgetBoard()
    /// Saved bug reports, kept from one window to the next.
    private let bugReports = BugReportsClient.inMemory()
    /// What the app last put on the clipboard.
    private let copied = LockIsolated<String?>(nil)

    private func app(
        _ name: String, workspace: Workspace? = nil, sync: SyncClient = .off, replies: [AssistantReply] = [.example],
        inputs: Set<ModelInput> = [], lock: LockClient = .always(),
        session: InMemorySession = InMemorySession(), assistant: AssistantClient? = nil,
        settings: Bool = false
    ) -> (StoreOf<RootFeature>, FlowRecorder) {
        let workspace = workspace ?? Workspace.sample(now: now, timeZone: .gmt)
        let ids = LockIsolated(1000)
        let (client, records) = StoreClient.inMemorySyncing(workspace, actor: .user(device: "Mac", surface: .mac))
        self.records = records
        let store = Store(initialState: RootFeature.State(platform: .mac)) {
            RootFeature()
        } withDependencies: { [time, widgets, bugReports, copied] in
            $0.store = client
            $0.session = session.client
            $0.reminders = RemindersClient.inMemory().client
            $0.sync = sync
            $0.assistant = assistant ?? .scripted(replies, inputs: inputs)
            $0.attachments = .inMemory()
            $0.recorder = .scripted(RecorderClient.Recording(data: Data([1]), seconds: 6))
            // The assistant never stalls in a flow.
            $0.continuousClock = TestClock()
            $0.conversations = .inMemory()
            $0.skills = .inMemory()
            $0.guidance = .inMemory()
            $0.models = InMemoryModelLibrary(installed: [ModelCatalog.gemma4E2B]).client
            $0.date = DateGenerator { time.value }
            $0.lock = lock
            $0.glance = GlanceClient { [widgets] glance in await widgets.show(glance) }
            $0.timeZone = .gmt
            $0.bugRecorder = BugRecorder(now: { time.value })
            $0.bugReports = bugReports
            $0.deviceInfo = .testValue
            $0.pasteboard = PasteboardClient { copied.setValue($0) }
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SeededRandomNumberGenerator())
            $0.uuid = UUIDGenerator {
                ids.withValue {
                    $0 += 1
                    return UUID($0)
                }
            }
        }
        let flow =
            settings
            ? FlowRecorder(
                name, MacSettingsView(store: store).environment(\.timeZone, .gmt),
                size: CGSize(width: 620, height: 680))
            : FlowRecorder(name, RootView(store: store).environment(\.timeZone, .gmt))
        store.send(.task)
        return (store, flow)
    }

    /// The live assistant on a model that gives `answers` in turn.
    /// Lifr picks `skill` for each message sent without one.
    private func gemma(_ answers: [JSONValue], picking skill: String = Skill.add.id) -> AssistantClient {
        let model = ScriptedRuntime(
            budget: ContextBudget(window: 32_768, reservedForAnswer: 2_048),
            steps: answers.map { ScriptedRuntime.answer($0) })
        let titles = ScriptedRuntime(steps: [ScriptedRuntime.answer("Groceries")])
        var client = AssistantClient.live(runtime: { use, _ in use == .assistant ? model : titles })
        client.pickSkill = { _, _, _, _ in skill }
        return client
    }

    private func send(_ text: String, in id: Conversation.ID, of store: StoreOf<RootFeature>) {
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.input, text))))))
        store.send(.conversations(.chats(.element(id: id, action: .sendTapped))))
    }

    /// Runs `body` as Shortcuts or Siri would, on the open app's records,
    /// at the app's time.
    private func shortcut<T>(_ body: (Shortcuts) async throws -> T) async throws -> T {
        let records = try XCTUnwrap(records, "Open the app first")
        let shortcuts = Shortcuts(
            store: records.writer(as: .user(device: "Mac", surface: .shortcuts)),
            locksApp: { Shared(.locksApp).wrappedValue }, now: { [time] in time.value }, timeZone: .gmt)
        let ids = LockIsolated(5000)
        return try await withDependencies { [time] in
            $0.date = DateGenerator { time.value }
            $0.uuid = UUIDGenerator {
                ids.withValue {
                    $0 += 1
                    return UUID($0)
                }
            }
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SeededRandomNumberGenerator())
        } operation: {
            try await body(shortcuts)
        }
    }

    private func task(_ title: String, in store: StoreOf<RootFeature>) throws -> UUID {
        try XCTUnwrap(store.workspace?.tasks.values.first { $0.title == title }?.id, "No task \(title)")
    }

    private func avenue(_ key: String, in store: StoreOf<RootFeature>) throws -> UUID {
        try XCTUnwrap(store.workspace?.avenues.values.first { $0.key.rawValue == key }?.id, "No Avenue \(key)")
    }

    /// Each list in the sidebar, then an Avenue as a list and a board.
    func testSidebar() async throws {
        let (store, flow) = app("sidebar")
        await flow.step("today")
        store.send(.selected(.upcoming))
        await flow.step("upcoming")
        store.send(.selected(.procrastinated))
        await flow.step("procrastinated")
        store.send(.selected(.search))
        store.send(.search(.binding(.set(\.filter.text, "run"))))
        await flow.step("search")
        store.send(.selected(.scope(.avenue(try avenue("HEALTH", in: store)))))
        await flow.step("avenue")
        store.content = .board
        await flow.step("board")
        flow.finish()
    }

    /// A task in the detail column: adding a Sub task, then finishing it
    /// with open Sub tasks.
    func testTaskDetail() async throws {
        let (store, flow) = app("detail")
        await flow.step("launch", image: false)
        let review = try task("Review Q4 roadmap", in: store)
        store.send(.today(.list(.rowTapped(review))))
        await flow.step("opened")
        store.send(.today(.list(.detail(.presented(.binding(.set(\.newSubtaskTitle, "Book a room")))))))
        store.send(.today(.list(.detail(.presented(.addSubtaskSubmitted)))))
        await flow.step("subtask-added")
        store.send(.statusShortcut(.done))
        await flow.step("finish-asks")
        flow.finish()
    }

    /// Quick add from ⌘N, and a new Avenue and Project from the sidebar.
    func testAdding() async throws {
        let (store, flow) = app("adding")
        await flow.step("launch", image: false)
        store.send(.newTaskTapped)
        store.send(.quickAdd(.presented(.binding(.set(\.title, "Paint the fence")))))
        await flow.step("quick-add")
        store.send(.quickAdd(.presented(.addTapped)))
        await flow.step("added")
        store.send(.newAvenueTapped)
        await flow.step("new-avenue")
        store.send(.avenues(.newAvenue(.dismiss)))
        store.send(.newProjectTapped(try avenue("HOME", in: store)))
        await flow.step("new-project")
        flow.finish()
    }

    /// Archiving a Project with its open task moved out, then ⌘Z, which
    /// puts both back.
    func testProjectUndo() async throws {
        let (store, flow) = app("project-undo")
        await flow.step("launch", image: false)
        let marathon = try XCTUnwrap(store.workspace?.projects.values.first { $0.name == "Half marathon" }?.id)
        store.send(.selected(.scope(.project(marathon))))
        await flow.step("opened", image: false)
        store.send(.avenue(.project(.archiveTapped)))
        store.send(.avenue(.project(.archive(.presented(.moveToAvenue)))))
        await flow.step("archived")
        let id = try XCTUnwrap(store.lastChange?.id)
        store.send(.undoTapped(id))
        await flow.step("undone")
        XCTAssertEqual(store.workspace?.projects[marathon]?.status, .active)
        XCTAssertEqual(
            store.workspace?.tasks.values.filter { $0.projectID == marathon && $0.isOpen }.map(\.title),
            ["Buy running shoes"])
        flow.finish()
    }

    /// The assistant: a new conversation, the skill finder, a reply with a
    /// draft, adding it, renaming, then the archive.
    func testAssistant() async throws {
        let (store, flow) = app("assistant")
        store.send(.selected(.conversations))
        await flow.step("list")
        store.send(.conversations(.newTapped))
        await flow.step("new")
        let id = try XCTUnwrap(store.conversations.openID)
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.input, "/"))))))
        await flow.step("slash")
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.input, ""))))))
        store.send(.conversations(.chats(.element(id: id, action: .findSkillTapped))))
        await flow.step("find-skill")
        store.send(.conversations(.chats(.element(id: id, action: .pickSkillTapped("review")))))
        await flow.step("skill-picked")
        store.send(.conversations(.chats(.element(id: id, action: .clearSkillTapped))))
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.input, "Tidy my weekend"))))))
        store.send(.conversations(.chats(.element(id: id, action: .sendTapped))))
        await flow.step("replied-with-draft")
        store.send(.conversations(.chats(.element(id: id, action: .editDraftTapped))))
        await flow.step("edit-draft")
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.editsDraft, false))))))
        await flow.step("edited", image: false)
        store.send(.conversations(.chats(.element(id: id, action: .addTapped))))
        await flow.step("added")
        store.send(.conversations(.chats(.element(id: id, action: .renameTapped))))
        await flow.step("rename")
        store.send(.conversations(.rename(.dismiss)))
        store.send(.conversations(.archiveTapped(id)))
        store.send(.conversations(.archiveShown(true)))
        await flow.step("archive")
        flow.finish()
    }

    /// Picking what one chat runs on: a new chat, its Model sheet before
    /// the first message, Gemma thinking Long with a larger window, then the chat.
    /// Settings keeps its default.
    func testChatModel() async throws {
        let (store, flow) = app("chat-model")
        store.send(.selected(.conversations))
        store.send(.conversations(.newTapped))
        await flow.step("new")
        let id = try XCTUnwrap(store.conversations.openID)
        store.send(.conversations(.chats(.element(id: id, action: .modelTapped))))
        await flow.step("defaults")
        let sheet = { (action: ChatModelFeature.Action) in
            _ = store.send(.conversations(.chats(.element(id: id, action: .model(.presented(action))))))
        }
        let gemma = ModelCatalog.gemma4E2B.id
        sheet(.binding(.set(\.model, gemma)))
        let long = ModelChoice(model: gemma, thinking: .long, window: 16_384)
        sheet(.binding(.set(\.choice, long)))
        await flow.step("picked")
        store.send(.conversations(.chats(.element(id: id, action: .model(.dismiss)))))
        await flow.step("chat")
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.context.model, long)
        flow.finish()
    }

    /// Asking about a photo and a recording, with a model that reads both:
    /// attaching each, sending them, the reply, then a model that can't.
    func testAssistantAttachments() async throws {
        let assistant = AssistantClient.scripted(
            [
                AssistantReply(text: "A Boston fern. Water it when the top of the soil is dry.", isComplete: true)
            ],
            inputs: [.image, .audio]
        )
        .failing([2: .cantRead(.audio)])
        let (store, flow) = app("assistant-attachments", assistant: assistant)
        store.send(.selected(.conversations))
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        await flow.step("new")
        store.send(.conversations(.chats(.element(id: id, action: .photoPicked(Data([1]))))))
        await flow.step("photo", image: false)
        store.send(.conversations(.chats(.element(id: id, action: .recordTapped))))
        await flow.step("recording")
        store.send(.conversations(.chats(.element(id: id, action: .recordTapped))))
        store.send(
            .conversations(.chats(.element(id: id, action: .binding(.set(\.input, "What plant is this?"))))))
        await flow.step("attached")
        store.send(.conversations(.chats(.element(id: id, action: .sendTapped))))
        await flow.step("replied")
        store.send(.conversations(.chats(.element(id: id, action: .recordTapped))))
        store.send(.conversations(.chats(.element(id: id, action: .recordTapped))))
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.input, "And this one?"))))))
        // Stopping attaches the recording once it's saved.
        for _ in 0..<100 where store.conversations.chats[id: id]?.canSend != true {
            try await Task.sleep(for: .milliseconds(10))
        }
        store.send(.conversations(.chats(.element(id: id, action: .sendTapped))))
        await flow.step("cant-read")
        XCTAssertNotNil(store.conversations.chats[id: id]?.failure)
        flow.finish()
    }

    /// The assistant failing to answer, then Retry writing the reply, the
    /// message edited to write it again beside the old, and a message too
    /// long to answer, which Retry can't help.
    func testAssistantFails() async throws {
        let assistant = AssistantClient.scripted([
            AssistantReply(text: "Start with the urgent tasks, then the ones due this week.", isComplete: true),
            AssistantReply(text: "Keep Saturday for the garden and Sunday free.", isComplete: true),
        ])
        .failing([1: .unreadable, 4: .tooLong])
        let (store, flow) = app("assistant-fails", assistant: assistant)
        store.send(.selected(.conversations))
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        func chat(_ action: ChatFeature.Action) { store.send(.conversations(.chats(.element(id: id, action: action)))) }
        send("Plan my week", in: id, of: store)
        await flow.step("unreadable")
        let asked = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last)
        XCTAssertNotNil(store.conversations.chats[id: id]?.failure)

        chat(.retryTapped(messageID: asked.id))
        await flow.step("retried")
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.messages.count, 2)

        time.withValue { $0 += 60 }
        chat(.editTapped(messageID: asked.id))
        await flow.step("editing")
        chat(.binding(.set(\.input, "Plan my weekend")))
        chat(.sendTapped)
        await flow.step("edited")
        let edited = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.first)
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.branches(at: edited.id).count, 2)

        send("Add a task for every day of the year", in: id, of: store)
        await flow.step("too-long")
        let long = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last)
        XCTAssertEqual(store.conversations.chats[id: id]?.canRetry(long), false)
        flow.finish()
    }

    /// Stop while a reply is written: Send turns to Stop, and Stop keeps
    /// what the reply had written, then the person can send again.
    func testAssistantStops() async throws {
        var assistant = AssistantClient.scripted([
            AssistantReply(text: "Keep Saturday for the garden.", isComplete: true)
        ])
        let scripted = assistant.reply
        let asked = LockIsolated(0)
        assistant.reply = { request in
            guard
                asked.withValue({
                    $0 += 1
                    return $0
                }) == 1
            else { return scripted(request) }
            return AssistantReplies { continuation in
                let task = Task {
                    continuation.yield(AssistantReply(text: "Start with the urgent tasks, then"))
                    try? await Task.sleep(for: .seconds(3600))
                    continuation.finish()
                }
                continuation.onTermination = { _ in task.cancel() }
            }
        }
        let (store, flow) = app("assistant-stops", assistant: assistant)
        store.send(.selected(.conversations))
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        func chat(_ action: ChatFeature.Action) { store.send(.conversations(.chats(.element(id: id, action: action)))) }
        send("Plan my week", in: id, of: store)
        await flow.step("writing")
        XCTAssertEqual(store.conversations.chats[id: id]?.canStop, true)

        chat(.stopTapped)
        await flow.step("stopped")
        XCTAssertEqual(store.conversations.chats[id: id]?.canStop, false)
        XCTAssertEqual(
            store.conversations.chats[id: id]?.conversation.messages.last?.text, "Start with the urgent tasks, then")

        time.withValue { $0 += 60 }
        send("And the weekend?", in: id, of: store)
        await flow.step("sent-again")
        XCTAssertEqual(
            store.conversations.chats[id: id]?.conversation.messages.last?.text, "Keep Saturday for the garden.")
        flow.finish()
    }

    /// A long conversation: the reply written without older messages
    /// offers Compact, which summarizes them; the summary is shown where
    /// it ends, and Forget Summary sends the messages again.
    func testAssistantCompacts() async throws {
        let assistant = AssistantClient.scripted([
            AssistantReply(text: "Start with the urgent tasks, then the ones due this week.", isComplete: true),
            AssistantReply(text: "Keep Saturday for the garden and Sunday free.", isComplete: true, leftOut: 1),
        ])
        let (store, flow) = app("assistant-compacts", assistant: assistant)
        store.send(.selected(.conversations))
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        func chat(_ action: ChatFeature.Action) { store.send(.conversations(.chats(.element(id: id, action: action)))) }
        send("Plan my week", in: id, of: store)
        await flow.step("planned")
        time.withValue { $0 += 60 }
        send("And the weekend?", in: id, of: store)
        await flow.step("offered")
        XCTAssertEqual(store.conversations.chats[id: id]?.offersCompact, true)

        chat(.compactTapped)
        await flow.step("compacted")
        let summary = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.summary)
        XCTAssertEqual(store.conversations.chats[id: id]?.offersCompact, false)

        chat(.forgetSummaryTapped(summary.through))
        await flow.step("forgot")
        XCTAssertNil(store.conversations.chats[id: id]?.conversation.summary)
        flow.finish()
    }

    /// A brain dump sent without a skill: Lifr picks Break down, the
    /// assistant asks what it needs to, and once answered plans a Project
    /// for the move, its tasks in order with steps, and a task on its own.
    func testAssistantBreaksDownABrainDump() async throws {
        let (store, flow) = app(
            "assistant-breakdown",
            assistant: gemma(
                [
                    ["message": "Is the move by a date, and is the deposit for the new flat or the old one?"],
                    [
                        "plan": [
                            "projects": [["name": "Move flat", "avenue": "HOME", "due": "2026-12-05"]],
                            "tasks": [
                                [
                                    "title": "Get the old deposit back", "place": "HOME/Move flat",
                                    "due": "2026-11-20",
                                    "subtasks": [["title": "Photograph each room"], ["title": "Email the landlord"]],
                                ],
                                ["title": "Book a van", "place": "HOME/Move flat", "due": "2026-11-27"],
                                [
                                    "title": "Pack", "place": "HOME/Move flat", "due": "2026-12-04",
                                    "subtasks": [["title": "Buy boxes"], ["title": "Pack the kitchen last"]],
                                ],
                                ["title": "Call mum", "place": "HOME"],
                            ],
                        ],
                        "message": "Here's the move as a Project, in order, and calling your mum on its own.",
                    ],
                ], picking: Skill.breakdown.id))
        store.send(.selected(.conversations))
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        await flow.step("new-chat", image: false)
        send("ugh I have to move flat, get the deposit sorted, pack, book a van, and call mum", in: id, of: store)
        await flow.step("asks")
        let asked = try XCTUnwrap(store.conversations.chats[id: id])
        XCTAssertEqual(asked.conversation.messages.first?.skill, Skill.breakdown.id)
        XCTAssertNil(asked.conversation.draft, "it asks before it plans")

        send("By 5 December, and it's the old flat's deposit", in: id, of: store)
        await flow.step("planned")
        let planned = try XCTUnwrap(store.conversations.chats[id: id])
        XCTAssertEqual(planned.conversation.draft?.projects.map(\.name), ["Move flat"])
        XCTAssertEqual(
            planned.conversation.draft?.tasks.map(\.title),
            ["Get the old deposit back", "Book a van", "Pack", "Call mum"])
        XCTAssertNil(planned.problem)
        store.send(.conversations(.chats(.element(id: id, action: .addTapped))))
        await flow.step("added")
        XCTAssertNotNil(store.workspace?.projects.values.first { $0.name == "Move flat" }, "the Project wasn't added")
        flow.finish()
    }

    /// Asking the assistant to rename the Half marathon and set when
    /// it's due: the change it proposes, Apply, then Undo on its reply.
    func testAssistantEditsAProject() async throws {
        let workspace = Workspace.sample(now: now, timeZone: .gmt)
        let marathon = try XCTUnwrap(workspace.projects.values.first { $0.name == "Half marathon" })
        let (store, flow) = app(
            "assistant-edits-project", workspace: workspace,
            assistant: gemma(
                [
                    [
                        "projectChanges": [
                            ["project": "HEALTH/Half marathon", "rename": "Spring half marathon", "due": "2026-12-06"]
                        ],
                        "message": "I renamed it to Spring half marathon, due 6 December.",
                    ]
                ], picking: Skill.change.id))
        store.send(.selected(.conversations))
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        await flow.step("new-chat", image: false)
        send("Rename the Half marathon to “Spring half marathon”, due 2026-12-06", in: id, of: store)
        await flow.step("proposed")
        XCTAssertEqual(store.workspace?.projects[marathon.id]?.name, "Half marathon", "nothing changes yet")
        let reply = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last?.id)

        store.send(.conversations(.chats(.element(id: id, action: .applyTapped(messageID: reply)))))
        await flow.step("edited")
        XCTAssertEqual(store.workspace?.projects[marathon.id]?.name, "Spring half marathon")
        XCTAssertEqual(store.workspace?.projects[marathon.id]?.dueDate?.description, "2026-12-06")

        store.send(.conversations(.chats(.element(id: id, action: .undoTapped(messageID: reply)))))
        await flow.step("undone")
        XCTAssertEqual(store.workspace?.projects[marathon.id]?.name, "Half marathon")
        XCTAssertEqual(store.workspace?.projects[marathon.id]?.dueDate, marathon.dueDate)
        flow.finish()
    }

    /// The assistant proposing to finish a task: Decline, which changes
    /// nothing, then asked again, Edit to In Progress instead, and Apply.
    func testAssistantProposalDeclinedAndEdited() async throws {
        let workspace = Workspace.sample(now: now, timeZone: .gmt)
        let plumber = try XCTUnwrap(workspace.tasks.values.first { $0.title == "Call the plumber" })
        let code = JSONValue.string(workspace.displayCode(of: plumber))
        let done: JSONValue = ["changes": [["code": code, "status": "done"]], "message": "I'd mark it done."]
        let (store, flow) = app(
            "assistant-proposal", workspace: workspace, assistant: gemma([done, done], picking: Skill.change.id))
        store.send(.selected(.conversations))
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        func chat(_ action: ChatFeature.Action) { store.send(.conversations(.chats(.element(id: id, action: action)))) }
        await flow.step("new-chat", image: false)
        send("I called the plumber", in: id, of: store)
        await flow.step("proposed")
        let first = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last?.id)
        chat(.declineTapped(messageID: first))
        await flow.step("declined")
        XCTAssertEqual(store.workspace?.tasks[plumber.id]?.status, plumber.status)

        send("Mark “Call the plumber” done", in: id, of: store)
        await flow.step("asked-again", image: false)
        let second = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last?.id)
        chat(.editProposalTapped(messageID: second))
        var edit = try XCTUnwrap(store.conversations.chats[id: id]?.editingProposal)
        edit.actions = [.task(TaskChange(code: workspace.displayCode(of: plumber), status: "in_progress"))]
        chat(.binding(.set(\.editingProposal, edit)))
        await flow.step("editing")
        chat(.editProposalDone)
        await flow.step("edited")
        chat(.applyTapped(messageID: second))
        await flow.step("applied")
        XCTAssertEqual(store.workspace?.tasks[plumber.id]?.status, .inProgress)
        flow.finish()
    }

    /// Help › Record Steps, finishing a task while it records, Stop,
    /// describing the bug, copying it without the task's title, saving it,
    /// then finding it in Settings › Bug Reports.
    func testBugReport() async throws {
        let (store, flow) = app("bug-report")
        await flow.step("launch", image: false)
        store.send(.recordStepsTapped)
        await flow.step("recording")
        store.send(.selected(.today))
        let water = try XCTUnwrap(store.workspace?.tasks.values.first { $0.title == "Water the plants" }?.id)
        store.send(.today(.list(.toggled(water))))
        await flow.step("acted")
        store.send(.stopRecordingTapped)
        await flow.step("describe")
        XCTAssertEqual(store.bugReport?.report.recording?.steps.isEmpty, false)
        store.send(.bugReport(.presented(.binding(.set(\.report.happened, "Finishing a task hid the list.")))))
        store.send(.bugReport(.presented(.binding(.set(\.report.expected, "The list stays.")))))
        await flow.step("described")
        store.send(.bugReport(.presented(.copyTapped)))
        await flow.step("copied", image: false)
        let text = try XCTUnwrap(copied.value)
        XCTAssertTrue(text.contains("Finishing a task hid the list."))
        XCTAssertFalse(text.contains("Water the plants"))
        store.send(.bugReport(.presented(.saveTapped)))
        await flow.step("saved", image: false)
        XCTAssertNil(store.bugReport)
        flow.finish()

        let (settings, list) = app("bug-report-list", settings: true)
        settings.send(.settings(.bugReportsTapped))
        await list.step("list")
        list.finish()
    }

    /// The Settings window, from ⌘,: Settings, then Activity.
    func testSettings() async {
        let (store, flow) = app("settings", settings: true)
        await flow.step("settings")
        store.send(.syncChanged(SyncState(status: .failed(.noAccount))))
        await flow.step("sync-failed")
        store.send(.settings(.activityTapped))
        await flow.step("activity")
        store.send(.settings(.activity(.presented(.olderTapped))))
        await flow.step("activity-older")
        store.send(.settings(.activity(.dismiss)))
        store.send(.settings(.modelsTapped))
        await flow.step("models")
        store.send(.settings(.models(.presented(.windowPicked(32_768, for: ModelCatalog.gemma4E2B.id)))))
        await flow.step("models-window")
        store.send(.settings(.models(.presented(.measureTapped(ModelCatalog.gemma4E2B.id)))))
        await flow.step("models-measured")
        store.send(.settings(.models(.presented(.backendPicked(.cpu, for: ModelCatalog.gemma4E2B.id)))))
        await flow.step("models-cpu")
        store.send(.settings(.models(.dismiss)))
        store.send(.settings(.instructionsTapped))
        await flow.step("instructions")
        store.send(.settings(.instructions(.presented(.binding(.set(\.draft, "Answer in Spanish."))))))
        store.send(.settings(.instructions(.presented(.saveTapped))))
        await flow.step("instructions-saved")
        store.send(.settings(.instructions(.presented(.useTapped(nil)))))
        await flow.step("instructions-default")
        flow.finish()
    }

    /// An app connecting through the CLI: the approval sheet.
    func testConnectingAnApp() async {
        let session = InMemorySession()
        let (_, flow) = app("connect", session: session)
        await flow.step("launch", image: false)
        session.request(
            ApprovalRequest(
                id: UUID(1), identity: "com.anthropic.claude-code", client: "Claude Code", method: .searchTasks,
                requestedAt: now))
        await flow.step("approval")
        flow.finish()
    }

    /// Two records changed on this device and the Mac in ways that couldn't
    /// merge: the banner on the sidebar, the review, a choice, then Later.
    func testSyncReview() async throws {
        let sample = Review.sample(now: now, timeZone: .gmt)
        let state = SyncState(lastSynced: now, review: sample.items)
        let sync = SyncClient(
            states: { AsyncStream { $0.yield(state) } }, sync: { state }, resolve: { _ in state }, changed: {})
        let (store, flow) = app("sync-review", workspace: sample.workspace, sync: sync)
        store.send(.syncChanged(state))
        await flow.step("banner")
        store.send(.syncReviewTapped)
        await flow.step("review")
        store.send(.syncReview(.presented(.chose(sample.items[0].id, .avenue, .theirs))))
        await flow.step("chosen")
        store.send(.syncReview(.presented(.laterTapped)))
        await flow.step("later")
        flow.finish()
    }
    /// The lock: on, coming back after a few minutes away to the locked
    /// window, a try Touch ID turns down, then in.
    func testLock() async {
        let (store, flow) = app(
            "lock", lock: .scripted([nil, .cancelled, .failed("Touch ID didn't recognise you")], biometry: .touchID))
        store.send(.settings(.lockToggled(true)))
        await flow.step("on", image: false)
        store.send(.backgrounded)
        time.setValue(now + 5 * 60)
        store.send(.foregrounded)
        await flow.step("locked")
        store.send(.unlockTapped)
        await flow.step("not-recognised")
        store.send(.unlockTapped)
        await flow.step("unlocked", image: false)
        flow.finish()
    }

    /// Shortcuts and Siri while the app is open: a task added there shows on
    /// Today, one finished there is ticked off, and with the lock on, Siri
    /// says how many but not what.
    func testShortcuts() async throws {
        let (store, flow) = app("shortcuts")
        let added = try await shortcut { try await $0.add("Buy oat milk", avenue: nil, due: .today) }
        XCTAssertEqual(added.name, "Buy oat milk")
        await flow.step("added")
        let water = try task("Water the plants", in: store)
        _ = try await shortcut { try await $0.finish(water) }
        await flow.step("finished")

        @Shared(.locksApp) var locksApp
        $locksApp.withLock { $0 = true }
        let today = try await shortcut { try await $0.today() }
        XCTAssertGreaterThan(today.count, 0)
        XCTAssertTrue(today.titles.isEmpty)
        XCTAssertFalse(TodaySummary(today).text.contains("Buy oat milk"))
        do {
            _ = try await shortcut { try await $0.finish(water) }
            XCTFail("Expected the lock to stop it")
        } catch {
            XCTAssertEqual(error as? ShortcutError, .locked)
        }
        flow.finish()
    }

    /// The widgets follow Today as it changes in the app and from Shortcuts,
    /// show only how many while the lock is on, and turn over at midnight
    /// with the app closed.
    func testWidgets() async throws {
        let (store, flow) = app("widgets")
        let home = FlowRecorder("widgets-home", WidgetBoardView(board: widgets), size: CGSize(width: 640, height: 480))
        await flow.step("today", image: false)
        await home.step("today")
        _ = try await shortcut { try await $0.add("Buy oat milk", avenue: nil, due: .today) }
        await flow.step("added-by-shortcut", image: false)
        await home.step("added-by-shortcut")
        store.send(.settings(.lockToggled(true)))
        await home.step("locked")
        XCTAssertEqual(widgets.glance?.rows, [])
        store.send(.settings(.lockToggled(false)))
        await home.step("unlocked")
        let midnight = try XCTUnwrap(widgets.glance?.until)
        widgets.date = midnight
        await home.step("after-midnight")
        let milk = try XCTUnwrap(widgets.glance?.at(midnight)?.rows.first { $0.title == "Buy oat milk" })
        XCTAssertTrue(milk.isOverdue)
        home.finish()
        flow.finish()
    }
}

/// What the app last gave the widgets, for a Home Screen that shows them.
@MainActor @Observable
private final class WidgetBoard {
    var glance: TodayGlance?
    /// When the Home Screen is looked at, as the widget's timeline would
    /// show it then; nil for when the app wrote it.
    var date: Date?

    nonisolated init() {}

    func show(_ glance: TodayGlance) {
        self.glance = glance
    }
}

/// The Today widget, small and medium, as they'd sit on the Home Screen.
private struct WidgetBoardView: View {
    var board: WidgetBoard

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack(spacing: 20) {
                widget(.small, width: 170)
                Spacer(minLength: 0)
            }
            widget(.medium, width: 364)
            Spacer(minLength: 0)
        }
        .padding(19)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color(white: 0.85))
    }

    private func widget(_ size: TodayWidgetView.Size, width: CGFloat) -> some View {
        TodayWidgetView(glance: board.date.map { board.glance?.at($0) } ?? board.glance, size: size)
            .padding(16)
            .frame(width: width, height: 170)
            .background(Color.white, in: RoundedRectangle(cornerRadius: 22))
    }
}
