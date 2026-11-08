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
import LifrSearchClient
import LifrSessionClient
import LifrShortcuts
import LifrStoreClient
import LifrSyncClient
import LifrWidgetViews
import SwiftUI
import XCTest

/// Each of a person's flows on the iPhone, through the live app on the
/// sample records as of 2026-11-03 10:00 UTC: one reference per step, the
/// accessibility tree and an image, so the screens can be read in order.
@MainActor
final class PhoneFlowSnapshotTests: XCTestCase {
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
    /// The app's clock, which only moves when a flow says, so the assistant
    /// never stalls.
    private let clock = TestClock()

    private func app(
        _ name: String, workspace: Workspace? = nil, sync: SyncClient = .off, replies: [AssistantReply] = [.example],
        inputs: Set<ModelInput> = [], lock: LockClient = .always(), assistant: AssistantClient? = nil,
        tab: RootFeature.Tab = .today, style: UIUserInterfaceStyle = .light
    ) -> (StoreOf<RootFeature>, FlowRecorder) {
        let workspace = workspace ?? Workspace.sample(now: now, timeZone: .gmt)
        let ids = LockIsolated(1000)
        let (client, records) = StoreClient.inMemorySyncing(workspace, actor: .user(device: "iPhone", surface: .iPhone))
        self.records = records
        var state = RootFeature.State(platform: .phone)
        // The tab the app opens on: Search opens with its field in the tab bar.
        state.tab = tab
        let store = Store(initialState: state) {
            RootFeature()
        } withDependencies: { [time, widgets, bugReports, copied, clock] in
            $0.store = client
            $0.session = InMemorySession().client
            $0.reminders = RemindersClient.inMemory().client
            $0.sync = sync
            $0.assistant = assistant ?? .scripted(replies, inputs: inputs)
            $0.attachments = .inMemory()
            $0.recorder = .scripted(RecorderClient.Recording(data: Data([1]), seconds: 6))
            $0.continuousClock = clock
            $0.relatedWords = .scripted(["garden", "lawn"])
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
        let flow = FlowRecorder(name, RootView(store: store).environment(\.timeZone, .gmt), style: style)
        store.send(.task)
        return (store, flow)
    }

    /// Runs `body` as Shortcuts or Siri would, on the open app's records,
    /// at the app's time.
    private func shortcut<T>(_ body: (Shortcuts) async throws -> T) async throws -> T {
        let records = try XCTUnwrap(records, "Open the app first")
        let shortcuts = Shortcuts(
            store: records.writer(as: .user(device: "iPhone", surface: .shortcuts)),
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

    /// Today, finishing a task with Undo, then a task with open Sub tasks.
    func testToday() async throws {
        let (store, flow) = app("today")
        await flow.step("launch")
        let water = try task("Water the plants", in: store)
        store.send(.today(.list(.toggled(water))))
        await flow.step("finished-with-undo")
        let review = try task("Review Q4 roadmap", in: store)
        store.send(.today(.list(.rowAction(review, .status(.done)))))
        await flow.step("finish-with-open-subtasks")
        flow.finish()
    }

    /// A task's detail: its fields, adding a Sub task, a comment.
    func testTaskDetail() async throws {
        let (store, flow) = app("detail")
        await flow.step("launch", image: false)
        let review = try task("Review Q4 roadmap", in: store)
        store.send(.today(.list(.rowTapped(review))))
        await flow.step("opened")
        store.send(.today(.list(.detail(.presented(.binding(.set(\.newSubtaskTitle, "Book a room")))))))
        store.send(.today(.list(.detail(.presented(.addSubtaskSubmitted)))))
        await flow.step("subtask-added")
        store.send(.today(.list(.detail(.presented(.statusPicked(.done))))))
        await flow.step("finish-asks")
        flow.finish()
    }

    /// Procrastinated, from Today.
    func testProcrastinated() async throws {
        let (store, flow) = app("procrastinated")
        await flow.step("launch", image: false)
        store.send(.today(.procrastinatedTapped))
        await flow.step("opened")
        flow.finish()
    }

    /// Quick add: a task with an Avenue, a date and a priority.
    func testQuickAdd() async throws {
        let (store, flow) = app("quick-add")
        await flow.step("launch", image: false)
        store.send(.newTaskTapped)
        await flow.step("empty")
        let home = try XCTUnwrap(store.workspace?.avenues.values.first { $0.key.rawValue == "HOME" }?.id)
        store.send(.quickAdd(.presented(.binding(.set(\.title, "Paint the fence")))))
        store.send(.quickAdd(.presented(.binding(.set(\.avenueID, home)))))
        store.send(.quickAdd(.presented(.binding(.set(\.due, .tomorrow)))))
        store.send(.quickAdd(.presented(.binding(.set(\.priority, .high)))))
        await flow.step("filled")
        store.send(.quickAdd(.presented(.addTapped)))
        await flow.step("added")
        flow.finish()
    }

    /// Upcoming.
    func testUpcoming() async {
        let (store, flow) = app("upcoming")
        store.tab = .upcoming
        await flow.step("opened")
        flow.finish()
    }

    /// Avenues: the list, one Avenue, a new Project, a new Avenue.
    func testAvenues() async throws {
        let (store, flow) = app("avenues")
        store.tab = .avenues
        await flow.step("list")
        let health = try XCTUnwrap(store.workspace?.avenues.values.first { $0.key.rawValue == "HEALTH" }?.id)
        store.send(.avenues(.opened(.avenue(health))))
        await flow.step("avenue")
        store.send(.avenues(.avenue(.presented(.newProjectTapped))))
        await flow.step("new-project")
        store.send(.avenues(.avenue(.presented(.newProjectCancelled))))
        store.send(.avenues(.newAvenueTapped))
        await flow.step("new-avenue")
        flow.finish()
    }

    /// A Project: rename it, give it a due date, archive it with its open
    /// task moved out and Undo that, archive it again, find it under
    /// Archived, then restore it. And edit its Avenue.
    func testProject() async throws {
        let sample = Workspace.sample(now: now, timeZone: .gmt)
        let marathon = try XCTUnwrap(sample.projects.values.first { $0.name == "Half marathon" }?.id)
        let health = try XCTUnwrap(sample.avenues.values.first { $0.key.rawValue == "HEALTH" }?.id)
        let (store, flow) = app("project", workspace: sample)
        store.tab = .avenues
        await flow.step("avenues")
        store.send(.avenues(.opened(.project(marathon))))
        await flow.step("opened")
        store.send(.avenues(.avenue(.presented(.project(.binding(.set(\.name, "Spring half marathon")))))))
        store.send(.avenues(.avenue(.presented(.project(.nameSubmitted)))))
        store.send(.avenues(.avenue(.presented(.project(.dueDatePicked(try CalendarDate("2027-04-11")))))))
        await flow.step("renamed-due")
        store.send(.avenues(.avenue(.presented(.project(.archiveTapped)))))
        await flow.step("archive-asks")
        store.send(.avenues(.avenue(.presented(.project(.archive(.presented(.moveToAvenue)))))))
        await flow.step("archived")
        let archived = try XCTUnwrap(store.toast?.id)
        store.send(.undoTapped(archived))
        await flow.step("archive-undone")
        XCTAssertEqual(store.workspace?.projects[marathon]?.status, .active)
        store.send(.avenues(.avenue(.presented(.project(.archiveTapped)))))
        store.send(.avenues(.avenue(.presented(.project(.archive(.presented(.moveToAvenue)))))))
        await flow.step("archived-again", image: false)
        store.send(.avenues(.avenue(.dismiss)))
        await flow.step("archived-list")
        store.send(.avenues(.opened(.project(marathon))))
        store.send(.avenues(.avenue(.presented(.project(.restoreTapped)))))
        await flow.step("restored")
        store.send(.avenues(.opened(.avenue(health))))
        store.send(.avenues(.avenue(.presented(.editAvenueTapped))))
        await flow.step("edit-avenue")
        flow.finish()
    }

    /// Search: words, then a due filter.
    func testSearch() async {
        let (store, flow) = app("search", tab: .search)
        await flow.step("empty")
        store.send(.search(.binding(.set(\.filter.text, "run"))))
        await flow.step("words")
        store.send(.search(.binding(.set(\.filter.due, .overdue))))
        await flow.step("overdue")
        flow.finish()
    }

    /// The assistant: a new conversation, a skill, a reply with a draft,
    /// adding it, renaming, then the list.
    func testAssistant() async throws {
        let (store, flow) = app("assistant")
        store.tab = .conversations
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
        store.send(.conversations(.closed))
        await flow.step("list-after")
        store.send(.conversations(.archiveTapped(id)))
        store.send(.conversations(.archiveShown(true)))
        await flow.step("archive")
        flow.finish()
    }

    /// Adding an Avenue from a chat, then asking for more in the same chat.
    /// The model's next draft repeats the Avenue just added and puts a task
    /// in an Avenue that isn't there. The draft leaves out what's already
    /// added, and removing the bad row in the editor lets the rest be added.
    func testAssistantFollowUpDraft() async throws {
        let fitness = Plan.AvenueDraft(key: "FIT", name: "Fitness", color: "green", symbol: "figure.run")
        let (store, flow) = app(
            "assistant-follow-up",
            replies: [
                AssistantReply(
                    text: "Here's a Fitness Avenue. Tap Add to make it.", plan: Plan(avenues: [fitness]),
                    isComplete: true),
                AssistantReply(
                    text: "I added Reading, and a run to start with.",
                    plan: Plan(
                        avenues: [fitness, .init(key: "READ", name: "Reading", color: "purple", symbol: "book")],
                        tasks: [.init(title: "Go for a run", place: "SPORTS")]),
                    isComplete: true),
            ])
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.input, "Add a Fitness Avenue"))))))
        store.send(.conversations(.chats(.element(id: id, action: .sendTapped))))
        await flow.step("first-draft")
        flow.tap("Add")
        await flow.step("first-added", image: false)
        XCTAssertNotNil(store.workspace?.avenues.values.first { $0.key.rawValue == "FIT" }, "Fitness wasn't added")

        store.send(
            .conversations(.chats(.element(id: id, action: .binding(.set(\.input, "Now add a Reading Avenue"))))))
        store.send(.conversations(.chats(.element(id: id, action: .sendTapped))))
        await flow.step("second-draft")
        let draft = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.draft)
        XCTAssertEqual(draft.avenues.map(\.key), ["READ"], "The draft still has the Avenue that was added")

        flow.tap("Edit")
        await flow.step("edit-second-draft")
        // The Reading row's Remove, then the run's.
        flow.tap("Remove", occurrence: 1)
        await flow.step("bad-row-removed")
        let chat = try XCTUnwrap(store.conversations.chats[id: id])
        XCTAssertNil(chat.problem, "Removing the row the problem was about left the problem")
        XCTAssertTrue(chat.canAdd)
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.editsDraft, false))))))
        flow.tap("Add")
        await flow.step("second-added")
        XCTAssertNotNil(store.workspace?.avenues.values.first { $0.key.rawValue == "READ" }, "Reading wasn't added")
        flow.finish()
    }

    /// The assistant as the app has it, on a model that gives these
    /// `answers` in turn: what Gemma 4 E2B answered on a device, so what
    /// Lifr does with them runs as it does there.
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

    private func draftTitles(_ id: Conversation.ID, in store: StoreOf<RootFeature>) -> [String] {
        store.conversations.chats[id: id]?.conversation.draft?.tasks.map(\.title) ?? []
    }

    /// Adding to a draft, then changing and removing one of its tasks. The
    /// model writes the draft's task again with the new one, and changes
    /// it by number with a priority of none it wasn't asked for, and the
    /// first task for "that".
    func testAssistantFollowUpEdits() async throws {
        let (store, flow) = app(
            "assistant-follow-up-edits",
            assistant: gemma([
                [
                    "plan": ["tasks": [["title": "buy milk", "place": "HOME"]]],
                    "message": "I added a task to buy milk in HOME.",
                ],
                [
                    "draftChanges": [["task": "1", "moveTo": "HOME", "rename": "", "priority": "none"]],
                    "plan": [
                        "tasks": [["title": "buy milk", "place": "HOME"], ["title": "buy eggs", "place": "HOME"]]
                    ],
                    "message": "I added a task to buy eggs in HOME.",
                ],
                // Gemma changes the first task for "that", the one added last.
                [
                    "draftChanges": [["task": "1", "due": "Friday", "priority": "none", "remove": false]],
                    "message": "I moved buy milk to Friday.",
                ],
                [
                    "draftChanges": [["task": "2", "remove": true, "priority": "none"]],
                    "message": "I took buy eggs out of the plan.",
                ],
            ]))
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        await flow.step("new-chat", image: false)
        send("Add a task to buy milk, in HOME", in: id, of: store)
        await flow.step("first-draft", image: false)

        send("Also add buy eggs", in: id, of: store)
        await flow.step("also-added")
        XCTAssertEqual(draftTitles(id, in: store), ["buy milk", "buy eggs"])

        send("Change that to Friday", in: id, of: store)
        await flow.step("changed")
        let draft = store.conversations.chats[id: id]?.conversation.draft
        XCTAssertEqual(draft?.tasks.map(\.due), [nil, "Friday"])
        XCTAssertEqual(draft?.tasks.map(\.priority), [Priority.none, .none])

        send("Remove the second one", in: id, of: store)
        await flow.step("removed")
        XCTAssertEqual(draftTitles(id, in: store), ["buy milk"])
        flow.tap("Add")
        await flow.step("added", image: false)
        XCTAssertEqual(store.workspace?.tasks.values.filter { $0.title == "buy milk" }.count, 1)
        flow.finish()
    }

    /// Drafts the model gets a little wrong, which Lifr puts right before
    /// they show: a new Project that's there already with a task restating
    /// its goal and blank dates, then a new Avenue whose task's steps have
    /// one twice and a due date of none, then a task with steps that the
    /// model writes again as a Project nobody asked for.
    func testAssistantRepairsDrafts() async throws {
        let (store, flow) = app(
            "assistant-repairs",
            assistant: gemma([
                [
                    "message": "I added Stretch after every run to your Half marathon project.",
                    "plan": [
                        "avenues": [],
                        "projects": [
                            [
                                "name": "Half marathon", "avenue": "HEALTH", "due": "",
                                "details": "Goal is to complete a half marathon.",
                            ]
                        ],
                        "tasks": [
                            [
                                "title": "Stretch after every run", "priority": "none", "due": "",
                                "place": "HEALTH/Half marathon",
                            ],
                            [
                                "title": "Run half marathon", "priority": "none", "due": "",
                                "place": "HEALTH/Half marathon",
                            ],
                        ],
                    ],
                ],
                [
                    "plan": [
                        "avenues": [["key": "GARDEN", "name": "Garden"]],
                        "tasks": [
                            [
                                "title": "Plant tomatoes", "place": "Garden/plant_tomatoes",
                                "subtasks": [
                                    ["title": "Buy seedlings", "due": "none"], ["title": "Dig the bed"],
                                    ["title": "buy seedlings"],
                                ],
                            ]
                        ],
                    ],
                    "message": "Here's a Garden Avenue with a task to plant tomatoes.",
                ],
                [
                    "message": "I added a task to plan the team offsite.",
                    "plan": [
                        "avenues": [["key": "WORK", "name": "Work"]],
                        "projects": [
                            [
                                "name": "Team Offsite Planning", "avenue": "WORK",
                                "tasks": [["title": "Book a venue"], ["title": "Send the invites"]],
                            ]
                        ],
                        "tasks": [
                            [
                                "title": "Plan the team offsite", "place": "WORK/Team Offsite",
                                "subtasks": [["title": "Book a venue"], ["title": "Send the invites"]],
                            ]
                        ],
                    ],
                ],
            ]))
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        await flow.step("new-chat", image: false)
        send("Add a task to stretch after every run to my Half marathon project", in: id, of: store)
        await flow.step("project-draft")
        let chat = try XCTUnwrap(store.conversations.chats[id: id])
        XCTAssertEqual(chat.conversation.draft?.tasks.map(\.title), ["Stretch after every run"])
        XCTAssertEqual(chat.conversation.draft?.projects, [])
        XCTAssertNil(chat.problem)
        flow.tap("Add")
        await flow.step("project-added", image: false)

        send("Make a Garden Avenue with a task to plant tomatoes: buy seedlings, dig the bed", in: id, of: store)
        await flow.step("garden-draft")
        let garden = try XCTUnwrap(store.conversations.chats[id: id])
        XCTAssertEqual(garden.conversation.draft?.tasks.map(\.place), ["GARDEN"])
        XCTAssertEqual(
            garden.conversation.draft?.tasks.first?.subtasks.map(\.title), ["Buy seedlings", "Dig the bed"])
        XCTAssertNil(garden.problem)
        flow.tap("Add")
        await flow.step("garden-added", image: false)
        XCTAssertNotNil(store.workspace?.avenues.values.first { $0.key.rawValue == "GARDEN" }, "Garden wasn't added")

        send("Add a task to plan the team offsite in WORK: book a venue, send the invites", in: id, of: store)
        await flow.step("offsite-draft")
        XCTAssertEqual(draftTitles(id, in: store), ["Plan the team offsite"])
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.draft?.projects, [])
        flow.finish()
    }

    /// Adding an urgent task, where the model first writes back only the
    /// WORK Avenue that's there already and says it added the task: Lifr
    /// asks again, and the draft has the task.
    func testAssistantAsksAgainForForgottenTasks() async throws {
        let (store, flow) = app(
            "assistant-forgotten-tasks",
            assistant: gemma([
                [
                    "plan": ["avenues": [["name": "Work", "key": "WORK"]]],
                    "message": "I have added an urgent task to your WORK avenue titled 'file my taxes'.",
                ],
                [
                    "plan": [
                        "tasks": [["title": "file my taxes", "place": "WORK", "priority": "urgent"]],
                        "avenues": [["key": "WORK", "name": "Work"]],
                    ],
                    "message": "I have added an urgent task to file my taxes under the WORK avenue for you.",
                ],
            ]))
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        // Once the chat has the records, so it knows WORK is there.
        await flow.step("new-chat", image: false)
        send("Add an urgent task to file my taxes, in WORK", in: id, of: store)
        await flow.step("draft")
        let chat = try XCTUnwrap(store.conversations.chats[id: id])
        XCTAssertEqual(chat.conversation.draft?.tasks.map(\.title), ["file my taxes"])
        XCTAssertEqual(chat.conversation.draft?.tasks.first?.priority, .urgent)
        XCTAssertEqual(chat.conversation.draft?.avenues, [])
        flow.tap("Add")
        await flow.step("added", image: false)
        let added = store.workspace?.tasks.values.first { $0.title == "file my taxes" }
        XCTAssertEqual(added?.priority, .urgent)
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
        store.tab = .conversations
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
        flow.tap("Add")
        await flow.step("added")
        XCTAssertNotNil(store.workspace?.projects.values.first { $0.name == "Move flat" }, "the Project wasn't added")
        flow.finish()
    }

    /// Pushing a task to next Monday, where the model works the date out
    /// as the Saturday before: the task is due the Monday the person named.
    func testAssistantReschedulesToTheDayNamed() async throws {
        let workspace = Workspace.sample(now: now, timeZone: .gmt)
        let plumber = try XCTUnwrap(workspace.tasks.values.first { $0.title == "Call the plumber" })
        let code = JSONValue.string(workspace.displayCode(of: plumber))
        let (store, flow) = app(
            "assistant-reschedule", workspace: workspace,
            assistant: gemma(
                [
                    [
                        "changes": [["code": code, "due": "2026-11-07"]],
                        "message": "I moved Call the plumber to next Monday.",
                    ]
                ], picking: Skill.change.id))
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        await flow.step("new-chat", image: false)
        send("Push “Call the plumber” to next Monday", in: id, of: store)
        await flow.step("proposed", image: false)
        let reply = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last?.id)
        store.send(.conversations(.chats(.element(id: id, action: .applyTapped(messageID: reply)))))
        await flow.step("rescheduled")
        let task = store.workspace?.tasks.values.first { $0.title == "Call the plumber" }
        XCTAssertEqual(task?.dueDate?.description, "2026-11-09")
        flow.finish()
    }

    /// Renaming a task, where the model writes the old title back as the
    /// new one and a status it already has: the task gets the quoted title.
    func testAssistantRenamesATask() async throws {
        let workspace = Workspace.sample(now: now, timeZone: .gmt)
        let plumber = try XCTUnwrap(workspace.tasks.values.first { $0.title == "Call the plumber" })
        let code = JSONValue.string(workspace.displayCode(of: plumber))
        let (store, flow) = app(
            "assistant-rename", workspace: workspace,
            assistant: gemma(
                [
                    [
                        "changes": [["code": code, "status": "todo", "rename": "Call the plumber"]],
                        "message": "I renamed the task to Call the electrician.",
                    ]
                ], picking: Skill.change.id))
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        // Once the chat has the records, so it knows the task.
        await flow.step("new-chat", image: false)
        send("Rename “Call the plumber” to “Call the electrician”", in: id, of: store)
        await flow.step("proposed", image: false)
        let reply = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last?.id)
        store.send(.conversations(.chats(.element(id: id, action: .applyTapped(messageID: reply)))))
        await flow.step("renamed")
        let titles = store.workspace?.tasks.values.map(\.title) ?? []
        XCTAssertTrue(titles.contains("Call the electrician"))
        XCTAssertFalse(titles.contains("Call the plumber"))
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
        store.tab = .conversations
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
        store.tab = .conversations
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

    /// Going back in a chat past a reply whose plan was added, keeping its
    /// tasks; writing again from there, which starts a second version; the
    /// log of every point; then forking the chat from the first reply.
    func testChatCheckpoints() async throws {
        let (store, flow) = app(
            "chat-checkpoints",
            replies: [
                AssistantReply(text: "Start with the urgent tasks, then the ones due this week.", isComplete: true),
                .example,
                AssistantReply(text: "Saturday morning suits the garden; leave the afternoon free.", isComplete: true),
            ])
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        func chat(_ action: ChatFeature.Action) { store.send(.conversations(.chats(.element(id: id, action: action)))) }
        func later() { time.withValue { $0 += 60 } }
        send("What should I do first?", in: id, of: store)
        await flow.step("first", image: false)
        later()
        send("Tidy my weekend", in: id, of: store)
        await flow.step("drafted", image: false)
        chat(.addTapped)
        await flow.step("added", image: false)
        later()

        chat(.backTapped)
        await flow.step("undo-asked")
        let back = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.back)
        chat(.alert(.presented(.go(to: back, undo: false))))
        await flow.step("rewound")
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.messages.count, 2)
        XCTAssertNotNil(store.conversations.chats[id: id]?.conversation.forward, "What came after is kept")

        send("Plan Saturday instead", in: id, of: store)
        await flow.step("second-version")
        let conversation = try XCTUnwrap(store.conversations.chats[id: id]?.conversation)
        let asked = try XCTUnwrap(conversation.messages.last { $0.role == .person })
        XCTAssertEqual(conversation.branches(at: asked.id).count, 2)
        chat(.branchTapped(messageID: asked.id, by: -1))
        await flow.step("first-version")
        let shownLast = store.conversations.chats[id: id]?.conversation.messages.last
        XCTAssertEqual(shownLast?.text, AssistantReply.example.text)

        // The Undo toast's gone by now, and the log's sheet covers it.
        if let toast = store.toast { store.send(.toastExpired(toast.id)) }
        chat(.binding(.set(\.showsCheckpoints, true)))
        await flow.step("log")
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.checkpoints.count, 3)
        let shown = store.conversations.chats[id: id]?.conversation.messages ?? []
        let reply = try XCTUnwrap(shown.dropFirst().first)
        chat(.forkTapped(messageID: reply.id))
        await flow.step("forked")
        let fork = try XCTUnwrap(store.conversations.openID)
        XCTAssertNotEqual(fork, id)
        XCTAssertEqual(
            store.conversations.chats[id: fork]?.conversation.messages.map(\.text), shown.prefix(2).map(\.text))

        // Searching the list finds the reply on the branch not shown, and
        // opens the chat on it.
        store.send(.conversations(.closed))
        store.send(.conversations(.searched("afternoon")))
        await flow.step("search")
        let found = try XCTUnwrap(store.conversations.found.first)
        XCTAssertEqual(store.conversations.found.map(\.id), [id])
        store.send(.conversations(.foundTapped(found.id, messageID: found.message?.id)))
        await flow.step("found")
        XCTAssertEqual(
            store.conversations.chats[id: id]?.conversation.messages.last?.text,
            "Saturday morning suits the garden; leave the afternoon free.")

        // Finding in the chat goes to the newest message with the words.
        chat(.findTapped)
        chat(.binding(.set(\.findQuery, "saturday")))
        await flow.step("find")
        XCTAssertEqual(store.conversations.chats[id: id]?.findMatches.count, 2)

        // Related words find what the words typed don't, once the person
        // pauses.
        chat(.findClosed)
        store.send(.conversations(.closed))
        store.send(.conversations(.searched("yard work")))
        await clock.advance(by: .seconds(1))
        await flow.step("related")
        XCTAssertEqual(store.conversations.found.map(\.isRelated), [true])
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
        store.tab = .conversations
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

    /// Picking what one chat runs on: a new chat, its Model sheet before
    /// the first message, Gemma thinking Long with a larger window, then the chat.
    /// Settings keeps its default.
    /// The chat in dark mode: its composer, the / list, a draft and the
    /// model sheet.
    func testAssistantInDarkMode() async throws {
        let (store, flow) = app("assistant-dark", style: .dark)
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.input, "/"))))))
        await flow.step("slash")
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.input, "Tidy my weekend"))))))
        store.send(.conversations(.chats(.element(id: id, action: .sendTapped))))
        await flow.step("replied-with-draft")
        store.send(.conversations(.chats(.element(id: id, action: .modelTapped))))
        await flow.step("model")
    }

    func testChatModel() async throws {
        let (store, flow) = app("chat-model")
        store.tab = .conversations
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

    /// A reply with what the model thought: closed under its chevron with
    /// how long it took on the GPU, then opened.
    func testAssistantThinking() async throws {
        let thinking = ModelThinking(
            text: """
                The person wants to water the plants this weekend. Saturday morning is free, \
                so a task for Saturday at HOME fits.
                """,
            duration: .milliseconds(4_200), processor: .gpu)
        let (store, flow) = app(
            "assistant-thinking",
            replies: [
                AssistantReply(
                    text: "I'd water them on Saturday morning.", plan: nil, isComplete: true, thinking: thinking)
            ])
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        send("When should I water the plants?", in: id, of: store)
        await flow.step("collapsed")
        let reply = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last)
        XCTAssertEqual(reply.thinking, thinking)
        store.send(.conversations(.chats(.element(id: id, action: .thinkingToggled(messageID: reply.id)))))
        await flow.step("expanded")
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
        store.tab = .conversations
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
        store.tab = .conversations
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
        store.tab = .conversations
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

    /// Shaking to record the steps to a bug: finishing a task while it
    /// records, shaking again to stop, describing it, copying it without
    /// the task's title, saving it, then finding it in Settings.
    func testBugReport() async throws {
        let (store, flow) = app("bug-report")
        await flow.step("launch", image: false)
        store.send(.shaken)
        XCTAssertTrue(store.isBugMenuShown)
        store.isBugMenuShown = false
        store.send(.recordStepsTapped)
        await flow.step("recording")
        let water = try task("Water the plants", in: store)
        store.send(.today(.list(.toggled(water))))
        await flow.step("acted")
        store.send(.shaken)
        await flow.step("describe")
        XCTAssertFalse(store.isRecordingSteps)
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
        store.isSettingsShown = true
        store.send(.settings(.bugReportsTapped))
        await flow.step("list")
        let saved = try await bugReports.load().reports
        XCTAssertEqual(saved.map(\.happened), ["Finishing a task hid the list."])
        flow.finish()
    }

    /// Settings, its Activity, Models (with a larger window for Gemma) and
    /// Skills.
    func testSettings() async {
        let (store, flow) = app("settings")
        store.isSettingsShown = true
        await flow.step("opened")
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
        flow.scrollToEnd()
        await flow.step("models-measured")
        store.send(.settings(.models(.presented(.backendPicked(.cpu, for: ModelCatalog.gemma4E2B.id)))))
        await flow.step("models-cpu")
        store.send(.settings(.models(.dismiss)))
        store.send(.settings(.skillsTapped))
        await flow.step("skills")
        store.send(.settings(.skills(.presented(.newTapped))))
        await flow.step("new-skill")
        store.send(.settings(.skills(.dismiss)))
        store.send(.settings(.instructionsTapped))
        await flow.step("instructions")
        store.send(.settings(.instructions(.presented(.binding(.set(\.draft, "Answer in Spanish."))))))
        store.send(.settings(.instructions(.presented(.saveTapped))))
        await flow.step("instructions-saved")
        store.send(.settings(.instructions(.presented(.useTapped(nil)))))
        await flow.step("instructions-default")
        flow.finish()
    }

    /// Two records changed on this device and the Mac in ways that couldn't
    /// merge: the banner on Today, the review, a choice, then Later.
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
    /// The lock: turning it on in Settings, coming back after a few
    /// minutes away to the locked app, a try Face ID turns down, then in.
    func testLock() async {
        let (store, flow) = app("lock", lock: .scripted([nil, .cancelled, .failed("Face ID didn't recognise you")]))
        store.isSettingsShown = true
        store.send(.settings(.lockToggled(true)))
        await flow.step("turned-on")
        store.isSettingsShown = false
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
    /// Today, one finished there is ticked off, Activity names Shortcuts,
    /// and with the lock on, Siri says how many but not what.
    func testShortcuts() async throws {
        let (store, flow) = app("shortcuts")
        let added = try await shortcut { try await $0.add("Buy oat milk", avenue: nil, due: .today) }
        XCTAssertEqual(added.name, "Buy oat milk")
        await flow.step("added")
        let water = try task("Water the plants", in: store)
        _ = try await shortcut { try await $0.finish(water) }
        await flow.step("finished")
        store.isSettingsShown = true
        store.send(.settings(.activityTapped))
        await flow.step("activity")
        store.send(.settings(.activity(.dismiss)))
        store.isSettingsShown = false

        @Shared(.locksApp) var locksApp
        $locksApp.withLock { $0 = true }
        let today = try await shortcut { try await $0.today() }
        let summary = TodaySummary(today).text
        XCTAssertGreaterThan(today.count, 0)
        XCTAssertTrue(today.titles.isEmpty)
        XCTAssertFalse(summary.contains("Buy oat milk"))
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
        let home = FlowRecorder("widgets-home", WidgetBoardView(board: widgets), size: CGSize(width: 402, height: 874))
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
