import ComposableArchitecture
import LifrAssistantClient
import LifrAssistantLive
import LifrCore
import LifrGlanceClient
import LifrLiteRT
import LifrLockClient
import LifrModelsClient
import LifrModelsLive
import LifrRemindersClient
import LifrRoot
import LifrSessionClient
import LifrSnapshotTesting
import LifrStoreClient
import LifrSyncClient
import SwiftUI
import XCTest

/// The assistant on a real model: Gemma 4 E2B on this Mac's GPU, through
/// LiteRT-LM and the live app, on the sample records as of 2026-11-03
/// 10:00 UTC.
///
/// The model's words differ from one GPU to the next, even taking the
/// likeliest token each time, so the trees show each run of them as
/// `‹model text›` and the steps that show them have no image. What the
/// test checks is that the screen moved on from "Thinking…" to a reply and
/// a draft, that adding the draft added tasks, and that Undo took them
/// away again.
@MainActor
final class AssistantOnRealModelTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    /// The longest the model gets to load and reply. A first load compiles
    /// the GPU's programs, which takes a while.
    private let replyTimeout: Duration = .seconds(600)

    /// The model Bazel downloaded, as the catalog pins it.
    private func installedModel() throws -> InstalledModel {
        let spec = ModelCatalog.gemma4E2B
        let file = try XCTUnwrap(spec.source?.files.first, "The catalog's Gemma 4 E2B has no file")
        XCTAssertEqual(
            file.digest, .sha256(TestModel.sha256),
            """
            The catalog and test_model.bzl pin different files. Pin both to the same commit and SHA-256, from \
            the model's page on Hugging Face, so this tests what the app downloads.
            """)
        let runfiles = try XCTUnwrap(
            ProcessInfo.processInfo.environment["TEST_SRCDIR"],
            "Run this with apps/lifr/testing/snapshot/run_mac.sh, which says where Bazel put the model.")
        let path = URL(fileURLWithPath: runfiles).appending(path: TestModel.runfile)
        XCTAssertEqual(path.lastPathComponent, file.path)
        return InstalledModel(spec: spec, directory: path.deletingLastPathComponent(), installedAt: now)
    }

    private func app(model: InstalledModel) -> (StoreOf<RootFeature>, FlowRecorder) {
        let ids = LockIsolated(1000)
        let (client, _) = StoreClient.inMemorySyncing(
            Workspace.sample(now: now, timeZone: .gmt), actor: .user(device: "Mac", surface: .mac))
        let host = ModelHost(load: { model, backend async throws(ModelRuntimeError) in
            try await LiteRTLMEngine.load(model, on: backend, greedy: true)
        })
        let store = Store(initialState: RootFeature.State(platform: .mac)) {
            RootFeature()
        } withDependencies: { [now] in
            $0.store = client
            $0.session = InMemorySession().client
            $0.reminders = RemindersClient.inMemory().client
            $0.sync = .off
            $0.assistant = .live(runtime: { _ in LiteRTRuntime(model: model, host: host) })
            $0.attachments = .inMemory()
            $0.continuousClock = ContinuousClock()
            $0.conversations = .inMemory()
            $0.skills = .inMemory()
            $0.guidance = .inMemory()
            $0.models = InMemoryModelLibrary(installed: [model.spec]).client
            $0.date = .constant(now)
            $0.lock = .always()
            $0.glance = GlanceClient { _ in }
            $0.timeZone = .gmt
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SeededRandomNumberGenerator())
            $0.uuid = UUIDGenerator {
                ids.withValue {
                    $0 += 1
                    return UUID($0)
                }
            }
        }
        let flow = FlowRecorder(
            "real-model", RootView(store: store).environment(\.timeZone, .gmt), redactsModelText: true)
        store.send(.task)
        return (store, flow)
    }

    /// Asking for a task: the reply and its draft, fixing the draft, adding
    /// it, then Undo.
    func testAReplyItsDraftAndUndo() async throws {
        let (store, flow) = app(model: try installedModel())
        defer { flow.finish() }
        try await until("the records load", timeout: .seconds(10)) { store.workspace != nil }
        store.send(.selected(.conversations))
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        await flow.step("new")

        try await send("Add a task to buy milk tomorrow, in HOME", in: id, of: store)
        let draft = try XCTUnwrap(
            store.conversations.chats[id: id]?.conversation.draft, "The model answered without a draft")

        // A small model's draft often can't be added as it is: a task with
        // no place, or a date it got wrong. The person fixes it in the
        // editor, keeping the model's titles, so what follows doesn't
        // depend on how well this GPU's model did.
        let fixed = Self.fixed(draft)
        XCTAssertGreaterThan(fixed.taskCount, 0, "The model's draft adds no task: \(draft)")
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.conversation.draft, fixed))))))
        let chat = try XCTUnwrap(store.conversations.chats[id: id])
        XCTAssertNil(chat.problem, "The fixed draft can't be added: \(chat.problem ?? "")")
        await flow.step("fixed-draft", image: false)

        let before = try liveTasks(in: store)
        store.send(.conversations(.chats(.element(id: id, action: .addTapped))))
        try await until("the draft is added", timeout: .seconds(10)) {
            store.conversations.chats[id: id]?.conversation.draft == nil && store.lastChange != nil
        }
        let added = try liveTasks(in: store).subtracting(before)
        XCTAssertEqual(added.count, fixed.taskCount, "Adding the draft added a different number of tasks")

        let last = try XCTUnwrap(store.lastChange)
        store.send(.undoTapped(last.id))
        try await until("Undo takes them away", timeout: .seconds(10)) { store.lastChange == nil }
        XCTAssertEqual(try liveTasks(in: store), before, "Undo left tasks the draft added")
        await flow.step("undone", image: false)
    }

    /// Sends `text` in the conversation `id` and waits for the whole reply,
    /// which must be words, not an error.
    private func send(_ text: String, in id: UUID, of store: StoreOf<RootFeature>) async throws {
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.input, text))))))
        store.send(.conversations(.chats(.element(id: id, action: .sendTapped))))
        try await until("the model replies to “\(text)”", timeout: replyTimeout) {
            store.conversations.chats[id: id].map { $0.replying == nil && !$0.reloading } ?? false
        }
        let chat = try XCTUnwrap(store.conversations.chats[id: id])
        XCTAssertNil(chat.alert, "The model couldn't reply to “\(text)”")
        let reply = try XCTUnwrap(chat.conversation.messages.last)
        XCTAssertEqual(reply.role, .assistant, "No reply to “\(text)”")
        XCTAssertFalse(reply.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, "The reply is empty")
    }

    /// `draft`'s tasks, those of its Projects too, each in Home with
    /// nothing else set.
    private static func fixed(_ draft: Plan) -> Plan {
        var fixed = draft
        fixed.avenues = []
        fixed.labels = []
        fixed.projects = []
        fixed.tasks = (draft.tasks + draft.projects.flatMap(\.tasks)).map { task in
            var task = task
            task.place = "HOME"
            task.parent = nil
            task.start = nil
            task.due = nil
            task.dueTime = nil
            task.labels = []
            task.reminders = []
            task.subtasks = []
            return task
        }
        return fixed
    }

    /// The tasks not deleted.
    private func liveTasks(in store: StoreOf<RootFeature>) throws -> Set<UUID> {
        let workspace = try XCTUnwrap(store.workspace, "The records aren't loaded")
        return Set(workspace.tasks.values.filter { $0.deletedAt == nil }.map(\.id))
    }

    /// Waits for `done`, failing after `timeout`.
    private func until(_ what: String, timeout: Duration, _ done: () -> Bool) async throws {
        let clock = ContinuousClock()
        let deadline = clock.now + timeout
        while !done() {
            guard clock.now < deadline else {
                XCTFail("Gave up waiting for \(what) after \(timeout)")
                throw CancellationError()
            }
            try await Task.sleep(for: .milliseconds(200))
        }
    }
}
