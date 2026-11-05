import ComposableArchitecture
import LifrAssistantClient
import LifrCore
import LifrRemindersClient
import LifrRoot
import LifrSessionClient
import LifrSnapshotTesting
import LifrStoreClient
import SwiftUI
import XCTest

/// Each of a person's flows on the Mac, through the live app on the sample
/// records as of 2026-11-03 10:00 UTC: one reference per step, the
/// accessibility tree and an image of every window, so the screens can be
/// read in order.
@MainActor
final class MacFlowSnapshotTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func app(
        _ name: String, replies: [AssistantReply] = [.example], session: InMemorySession = InMemorySession(),
        settings: Bool = false
    ) -> (StoreOf<RootFeature>, FlowRecorder) {
        let workspace = Workspace.sample(now: now, timeZone: .gmt)
        let ids = LockIsolated(1000)
        let store = Store(initialState: RootFeature.State(platform: .mac)) {
            RootFeature()
        } withDependencies: { [now] in
            $0.store = .inMemory(workspace, actor: .user(device: "Mac", surface: .mac))
            $0.session = session.client
            $0.reminders = RemindersClient.inMemory().client
            $0.assistant = .scripted(replies)
            // The assistant never stalls in a flow.
            $0.continuousClock = TestClock()
            $0.conversations = .inMemory()
            $0.skills = .inMemory()
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
        let flow =
            settings
            ? FlowRecorder(
                name, MacSettingsView(store: store).environment(\.timeZone, .gmt),
                size: CGSize(width: 620, height: 680))
            : FlowRecorder(name, RootView(store: store).environment(\.timeZone, .gmt))
        store.send(.task)
        return (store, flow)
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
        store.send(.settings(.activityTapped))
        await flow.step("activity")
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
}
