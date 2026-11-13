import ComposableArchitecture
import FlowKit
import InferenceKitLiteRT
import LifrAPI
import LifrAssistantClient
import LifrAssistantLive
import LifrConversations
import LifrCore
import LifrGlanceClient
import LifrLockClient
import LifrModelsClient
import LifrModelsLive
import LifrRemindersClient
import LifrRoot
import LifrSessionClient
import LifrStoreClient
import LifrSyncClient
import XCTest

/// The live app on Gemma 4 E2B on this Mac's GPU, on the sample records as
/// of 2026-11-03 10:00 UTC, a Tuesday.
@MainActor
final class RealModel {
    static let now = Date(timeIntervalSince1970: 1_793_700_000)
    /// The longest the model gets to load and reply. A first load compiles
    /// the GPU's programs, which takes a while.
    static let replyTimeout: Duration = .seconds(600)

    let store: StoreOf<RootFeature>
    /// The conversation `chat()` opened.
    private(set) var conversation: UUID?

    init() throws {
        let model = try Self.installedModel()
        let ids = LockIsolated(1000)
        let (client, _) = StoreClient.inMemorySyncing(
            Workspace.sample(now: Self.now, timeZone: .gmt), actor: .user(device: "Mac", surface: .mac))
        let host = ModelHost(load: { model, backend async throws(ModelRuntimeError) in
            try await LiteRTLMEngine.load(model, on: backend, greedy: true)
        })
        store = Store(initialState: RootFeature.State(platform: .mac)) {
            RootFeature()
        } withDependencies: {
            $0.store = client
            $0.session = InMemorySession().client
            $0.reminders = RemindersClient.inMemory().client
            $0.sync = .off
            $0.assistant = .live(runtime: { _, _ in LoggedRuntime(base: EngineRuntime(model: model, host: host)) })
            $0.attachments = .inMemory()
            $0.continuousClock = ContinuousClock()
            $0.conversations = .inMemory()
            $0.skills = .inMemory()
            $0.guidance = .inMemory()
            $0.models = InMemoryModelLibrary(installed: [model.spec]).client
            $0.date = .constant(Self.now)
            $0.lock = .always()
            $0.glance = GlanceClient(write: { _ in }, takeMarked: { [] })
            $0.timeZone = .gmt
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SeededRandomNumberGenerator())
            $0.uuid = UUIDGenerator {
                ids.withValue {
                    $0 += 1
                    return UUID($0)
                }
            }
        }
        store.send(.task)
    }

    /// The model Bazel downloaded, as the catalog pins it.
    static func installedModel() throws -> InstalledModel {
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
            "Run this with apps/libs/kits/flow/run_mac.sh, which says where Bazel put the model.")
        let path = URL(fileURLWithPath: runfiles).appending(path: TestModel.runfile)
        XCTAssertEqual(path.lastPathComponent, file.path)
        return InstalledModel(spec: spec, directory: path.deletingLastPathComponent(), installedAt: now)
    }

    /// Waits for the records, then opens a new conversation.
    @discardableResult
    func chat() async throws -> UUID {
        try await until("the records load", timeout: .seconds(10)) { store.workspace != nil }
        store.send(.selected(.conversations))
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        conversation = id
        return id
    }

    var chatState: ChatFeature.State? {
        conversation.flatMap { store.conversations.chats[id: $0] }
    }

    var draft: Plan? { chatState?.conversation.draft }

    var workspace: Workspace {
        get throws { try XCTUnwrap(store.workspace, "The records aren't loaded") }
    }

    /// Sends `text` in the open conversation and waits for the whole reply,
    /// which must be words, not an error.
    func send(_ text: String, file: StaticString = #filePath, line: UInt = #line) async throws {
        let id = try XCTUnwrap(conversation, "Open a chat first", file: file, line: line)
        store.send(.conversations(.chats(.element(id: id, action: .core(.binding(.set(\.input, text)))))))
        store.send(.conversations(.chats(.element(id: id, action: .core(.sendTapped)))))
        try await until("the model replies to “\(text)”", timeout: Self.replyTimeout) {
            store.conversations.chats[id: id].map { $0.core.reply == nil && !$0.reloading } ?? false
        }
        let chat = try XCTUnwrap(store.conversations.chats[id: id])
        XCTAssertNil(chat.alert, "The model couldn't reply to “\(text)”", file: file, line: line)
        let reply = try XCTUnwrap(chat.conversation.messages.last)
        XCTAssertEqual(reply.role, .assistant, "No reply to “\(text)”", file: file, line: line)
        let skill = chat.conversation.messages.last { $0.role == .person }?.skill ?? "none"
        let draft = chat.conversation.draft.map(PlanFile.write) ?? "none"
        print("“\(text)” [\(skill)] → \(reply.text)\nDraft: \(draft)")
    }

    /// Adds the draft as the model left it, and returns the tasks it added.
    func addDraft(file: StaticString = #filePath, line: UInt = #line) async throws -> [LifrTask] {
        let id = try XCTUnwrap(conversation, "Open a chat first", file: file, line: line)
        let chat = try XCTUnwrap(chatState)
        let draft = try XCTUnwrap(
            chat.conversation.draft,
            "The model answered without a draft: \(chat.conversation.messages.last?.text ?? "")",
            file: file, line: line)
        if let problem = chat.problem {
            XCTFail("The draft can't be added: \(problem)\n\(PlanFile.write(draft))", file: file, line: line)
            throw CancellationError()
        }
        let before = try liveTasks()
        store.send(.conversations(.chats(.element(id: id, action: .addTapped))))
        try await until("the draft is added", timeout: .seconds(10)) { self.currentDraft(in: id) == nil }
        let after = try workspace
        return after.tasks.values.filter { $0.deletedAt == nil && !before.contains($0.id) }
            .sorted { $0.code < $1.code }
    }

    /// Applies what the last reply asks to change, as tapping Apply on its
    /// card does, and waits for it to be saved.
    func applyProposal(file: StaticString = #filePath, line: UInt = #line) async throws {
        let id = try XCTUnwrap(conversation, "Open a chat first", file: file, line: line)
        let reply = try XCTUnwrap(chatState?.conversation.messages.last, "No reply", file: file, line: line)
        _ = try XCTUnwrap(
            chatState?.proposal(of: reply.id), "The model asked to change nothing: \(reply.text)", file: file,
            line: line)
        store.send(.conversations(.chats(.element(id: id, action: .applyTapped(messageID: reply.id)))))
        try await until("the changes are applied", timeout: .seconds(10)) {
            self.chatState.map { $0.applying == nil && $0.proposal(of: reply.id)?.decision != nil } ?? false
        }
        XCTAssertNil(chatState?.alert, "The changes couldn't be applied", file: file, line: line)
    }

    private func currentDraft(in id: UUID) -> Plan? { store.conversations.chats[id: id]?.conversation.draft }

    /// The IDs of the tasks not deleted.
    func liveTasks() throws -> Set<UUID> {
        Set(try workspace.tasks.values.filter { $0.deletedAt == nil }.map(\.id))
    }

    /// The live task whose title has `words`, ignoring case.
    func task(titled words: String, file: StaticString = #filePath, line: UInt = #line) throws -> LifrTask {
        try XCTUnwrap(
            try workspace.tasks.values.first {
                $0.deletedAt == nil && $0.title.localizedCaseInsensitiveContains(words)
            },
            "No task titled “\(words)”", file: file, line: line)
    }

    func avenue(of task: LifrTask) throws -> String? {
        try workspace.avenues[task.avenueID]?.key.rawValue
    }

    /// Waits up to a few seconds for `done`, without failing: an applied
    /// change shows in the app's records once they reload.
    func settled(_ done: () throws -> Bool) async {
        let deadline = ContinuousClock().now + .seconds(5)
        while ContinuousClock().now < deadline, (try? done()) != true {
            try? await Task.sleep(for: .milliseconds(100))
        }
    }

    /// Waits for `done`, failing after `timeout`.
    func until(_ what: String, timeout: Duration, _ done: () -> Bool) async throws {
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
