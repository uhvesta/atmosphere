import ComposableArchitecture
import LifrAssistantClient
import LifrCore
import LifrGlance
import LifrGlanceClient
import LifrLockClient
import LifrModelsClient
import LifrRemindersClient
import LifrRoot
import LifrSessionClient
import LifrShortcuts
import LifrSnapshotTesting
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

    private func app(
        _ name: String, workspace: Workspace? = nil, sync: SyncClient = .off, replies: [AssistantReply] = [.example],
        inputs: Set<ModelInput> = [], lock: LockClient = .always()
    ) -> (StoreOf<RootFeature>, FlowRecorder) {
        let workspace = workspace ?? Workspace.sample(now: now, timeZone: .gmt)
        let ids = LockIsolated(1000)
        let (client, records) = StoreClient.inMemorySyncing(workspace, actor: .user(device: "iPhone", surface: .iPhone))
        self.records = records
        let store = Store(initialState: RootFeature.State(platform: .phone)) {
            RootFeature()
        } withDependencies: { [time, widgets] in
            $0.store = client
            $0.session = InMemorySession().client
            $0.reminders = RemindersClient.inMemory().client
            $0.sync = sync
            $0.assistant = .scripted(replies, inputs: inputs)
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
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SeededRandomNumberGenerator())
            $0.uuid = UUIDGenerator {
                ids.withValue {
                    $0 += 1
                    return UUID($0)
                }
            }
        }
        let flow = FlowRecorder(name, RootView(store: store).environment(\.timeZone, .gmt))
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

    /// Search: words, then a due filter.
    func testSearch() async {
        let (store, flow) = app("search")
        store.tab = .search
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
                        tasks: [.init(title: "Go for a run", place: "FITNESS")]),
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

    /// Asking about a photo and a recording, with a model that reads both:
    /// attaching each, sending them, the reply, then a model that can't.
    func testAssistantAttachments() async throws {
        let (store, flow) = app(
            "assistant-attachments",
            replies: [
                AssistantReply(text: "A Boston fern. Water it when the top of the soil is dry.", isComplete: true)
            ],
            inputs: [.image, .audio])
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
        store.send(
            .conversations(.chats(.element(id: id, action: .replyFailed(AssistantError.cantRead(.audio))))))
        await flow.step("cant-read")
        flow.finish()
    }

    /// The assistant failing: an answer it couldn't read.
    func testAssistantFails() async throws {
        let (store, flow) = app("assistant-fails")
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.input, "Plan my week"))))))
        store.send(.conversations(.chats(.element(id: id, action: .sendTapped))))
        store.send(
            .conversations(.chats(.element(id: id, action: .replyFailed(AssistantError.unreadable(saved: false))))))
        await flow.step("unreadable")
        for (name, error) in [
            ("timed-out", AssistantError.timedOut(saved: false)), ("too-long", AssistantError.tooLong(saved: false)),
        ] {
            store.send(.conversations(.chats(.element(id: id, action: .alert(.dismiss)))))
            await flow.step("dismissed", image: false)
            store.send(.conversations(.chats(.element(id: id, action: .replyFailed(error)))))
            await flow.step(name)
        }
        flow.finish()
    }

    /// Settings, its Activity, Models and Skills.
    func testSettings() async {
        let (store, flow) = app("settings")
        store.isSettingsShown = true
        await flow.step("opened")
        store.send(.syncChanged(SyncState(status: .failed(.noAccount))))
        await flow.step("sync-failed")
        store.send(.settings(.activityTapped))
        await flow.step("activity")
        store.send(.settings(.activity(.dismiss)))
        store.send(.settings(.modelsTapped))
        await flow.step("models")
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
    /// and show only how many while the lock is on.
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
        home.finish()
        flow.finish()
    }
}

/// What the app last gave the widgets, for a Home Screen that shows them.
@MainActor @Observable
private final class WidgetBoard {
    var glance: TodayGlance?

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
        TodayWidgetView(glance: board.glance, size: size)
            .padding(16)
            .frame(width: width, height: 170)
            .background(Color.white, in: RoundedRectangle(cornerRadius: 22))
    }
}
