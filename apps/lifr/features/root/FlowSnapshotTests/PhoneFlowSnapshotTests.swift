import ComposableArchitecture
import LifrAssistantClient
import LifrCore
import LifrRemindersClient
import LifrRoot
import LifrSessionClient
import LifrSnapshotTesting
import LifrStoreClient
import LifrSyncClient
import SwiftUI
import XCTest

/// Each of a person's flows on the iPhone, through the live app on the
/// sample records as of 2026-11-03 10:00 UTC: one reference per step, the
/// accessibility tree and an image, so the screens can be read in order.
@MainActor
final class PhoneFlowSnapshotTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func app(
        _ name: String, workspace: Workspace? = nil, sync: SyncClient = .off, replies: [AssistantReply] = [.example]
    ) -> (StoreOf<RootFeature>, FlowRecorder) {
        let workspace = workspace ?? Workspace.sample(now: now, timeZone: .gmt)
        let ids = LockIsolated(1000)
        let store = Store(initialState: RootFeature.State(platform: .phone)) {
            RootFeature()
        } withDependencies: { [now] in
            $0.store = .inMemory(workspace, actor: .user(device: "iPhone", surface: .iPhone))
            $0.session = InMemorySession().client
            $0.reminders = RemindersClient.inMemory().client
            $0.sync = sync
            $0.assistant = .scripted(replies)
            // The assistant never stalls in a flow.
            $0.continuousClock = TestClock()
            $0.conversations = .inMemory()
            $0.skills = .inMemory()
            $0.guidance = .inMemory()
            $0.date = DateGenerator { now }
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

    /// Settings, its Activity and Skills.
    func testSettings() async {
        let (store, flow) = app("settings")
        store.isSettingsShown = true
        await flow.step("opened")
        store.send(.syncChanged(SyncState(status: .failed(.noAccount))))
        await flow.step("sync-failed")
        store.send(.settings(.activityTapped))
        await flow.step("activity")
        store.send(.settings(.activity(.dismiss)))
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
}
