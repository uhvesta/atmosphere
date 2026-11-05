import ComposableArchitecture
import LifrAssistantClient
import LifrCore
import LifrLockClient
import LifrModelsClient
import LifrRemindersClient
import LifrRoot
import LifrSessionClient
import LifrShortcuts
import LifrSnapshotTesting
import LifrStoreClient
import LifrSyncClient
import SwiftUI
import XCTest

/// Each of a person's flows on the Mac, through the live app on the sample
/// records as of 2026-11-03 10:00 UTC: one reference per step, the
/// accessibility tree and an image of every window, so the screens can be
/// read in order.
@MainActor
final class MacFlowSnapshotTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    /// The time the app thinks it is, which a flow can move on.
    private let time = LockIsolated(Date(timeIntervalSince1970: 1_793_700_000))
    /// The app's records, for another writer such as Shortcuts.
    private var records: InMemorySync?

    private func app(
        _ name: String, workspace: Workspace? = nil, sync: SyncClient = .off, replies: [AssistantReply] = [.example],
        inputs: Set<ModelInput> = [], lock: LockClient = .always(),
        session: InMemorySession = InMemorySession(),
        settings: Bool = false
    ) -> (StoreOf<RootFeature>, FlowRecorder) {
        let workspace = workspace ?? Workspace.sample(now: now, timeZone: .gmt)
        let ids = LockIsolated(1000)
        let (client, records) = StoreClient.inMemorySyncing(workspace, actor: .user(device: "Mac", surface: .mac))
        self.records = records
        let store = Store(initialState: RootFeature.State(platform: .mac)) {
            RootFeature()
        } withDependencies: { [time] in
            $0.store = client
            $0.session = session.client
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
            $0.timeZone = .gmt
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

    /// The assistant: a new conversation, the skill finder, a reply with a
    /// draft, adding it, renaming, then the archive.
    func testAssistant() async throws {
        let (store, flow) = app("assistant")
        store.send(.selected(.conversations))
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
        store.send(.conversations(.archiveTapped(id)))
        store.send(.conversations(.archiveShown(true)))
        await flow.step("archive")
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
        store.send(
            .conversations(.chats(.element(id: id, action: .replyFailed(AssistantError.cantRead(.audio))))))
        await flow.step("cant-read")
        flow.finish()
    }

    /// The assistant failing: an answer it couldn't read.
    func testAssistantFails() async throws {
        let (store, flow) = app("assistant-fails")
        store.send(.selected(.conversations))
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

    /// The Settings window, from ⌘,: Settings, then Activity.
    func testSettings() async {
        let (store, flow) = app("settings", settings: true)
        await flow.step("settings")
        store.send(.syncChanged(SyncState(status: .failed(.noAccount))))
        await flow.step("sync-failed")
        store.send(.settings(.activityTapped))
        await flow.step("activity")
        store.send(.settings(.activity(.dismiss)))
        store.send(.settings(.modelsTapped))
        await flow.step("models")
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
}
