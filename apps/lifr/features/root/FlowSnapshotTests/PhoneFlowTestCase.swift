import CaptureKit
import ChatFeatures
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
import LifrPasteboardClient
import LifrRemindersClient
import LifrRoot
import LifrSearchClient
import LifrSessionClient
import LifrShortcuts
import LifrStoreClient
import LifrSyncClient
import LifrUI
import LifrWidgetViews
import SwiftUI
import XCTest

/// Each of a person's flows on the iPhone, through the live app on the
/// sample records as of 2026-11-03 10:00 UTC: one reference per step, the
/// accessibility tree and an image, so the screens can be read in order.
/// Each area's flows subclass it, in a test of their own, so the areas run
/// side by side.
@MainActor
class PhoneFlowTestCase: XCTestCase {
    let now = Date(timeIntervalSince1970: 1_793_700_000)
    /// The time the app thinks it is, which a flow can move on.
    let time = LockIsolated(Date(timeIntervalSince1970: 1_793_700_000))
    /// The app's records, for another writer such as Shortcuts.
    var records: InMemorySync?
    /// What the app last gave the widgets.
    let widgets = WidgetBoard()
    /// Saved bug reports, kept from one window to the next.
    let bugReports = BugReportsClient.inMemory()
    /// What the app last put on the clipboard.
    let copied = LockIsolated<String?>(nil)
    /// The app's clock, which only moves when a flow says, so the assistant
    /// never stalls.
    let clock = TestClock()
    /// The models installed: one that reads only text, and for a flow with
    /// photos one that also reads them, which `seePhotos` runs the assistant on.
    var library = InMemoryModelLibrary()

    func app(
        _ name: String, workspace: Workspace? = nil, sync: SyncClient = .off, replies: [AssistantReply] = [.example],
        inputs: Set<ModelInput> = [], readsPhotos: Bool = false, lock: LockClient = .always(),
        assistant: AssistantClient? = nil,
        photos: PhotoLibraryClient = .denied, files: [String: PickedFile] = [:],
        tab: RootFeature.Tab = .today, style: UIUserInterfaceStyle = .light, speech: SpeechClient = .inMemory,
        tips: Bool = false, textSize: DynamicTypeSize = .large
    ) -> (StoreOf<RootFeature>, FlowRecorder) {
        // The gesture tips show only in a flow about them.
        for tip in GestureTip.allCases {
            Shared(wrappedValue: false, .appStorage(tip.storageKey)).withLock { $0 = !tips }
        }
        let workspace = workspace ?? Workspace.sample(now: now, timeZone: .gmt)
        let ids = LockIsolated(1000)
        let library = InMemoryModelLibrary(
            installed: [ModelCatalog.gemma4E2B]
                + (readsPhotos || inputs.contains(.image) ? [ModelCatalog.gemma4E4B] : []))
        self.library = library
        let (client, records) = StoreClient.inMemorySyncing(workspace, actor: .user(device: "iPhone", surface: .iPhone))
        self.records = records
        var state = RootFeature.State(platform: .phone)
        // The tab the app opens on: Search opens with its field in the tab bar.
        state.tab = tab
        let store = Store(initialState: state) {
            RootFeature()
        } withDependencies: { [time, widgets, bugReports, copied, clock, library] in
            $0.store = client
            $0.session = InMemorySession().client
            $0.reminders = RemindersClient.inMemory().client
            $0.sync = sync
            $0.assistant = assistant ?? .scripted(replies, inputs: inputs)
            $0.attachments = .inMemory()
            $0.recorder = .scripted(RecorderClient.Recording(data: Data([1]), seconds: 6))
            $0.dictation = .scripted(["Move my run", "Move my run to Friday"])
            $0.photoLibrary = photos
            $0.fileReader = .scripted(files)
            $0.continuousClock = clock
            $0.relatedWords = .scripted(["garden", "lawn"])
            $0.conversations = .inMemory()
            $0.skills = .inMemory()
            $0.guidance = .inMemory()
            $0.models = library.client
            $0.speech = speech
            $0.date = DateGenerator { time.value }
            $0.lock = lock
            $0.glance = GlanceClient { [widgets] glance in await widgets.show(glance) }
            $0.timeZone = .gmt
            $0.bugRecorder = BugRecorder(now: { time.value })
            $0.bugReports = bugReports
            $0.deviceInfo = .testValue
            $0.pasteboard = PasteboardClient {
                copied.setValue($0)
            } read: {
                copied.value
            }
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SeededRandomNumberGenerator())
            $0.uuid = UUIDGenerator {
                ids.withValue {
                    $0 += 1
                    return UUID($0)
                }
            }
        }
        let flow = FlowRecorder(
            name, RootView(store: store).environment(\.timeZone, .gmt).dynamicTypeSize(textSize), style: style,
            audits: true)
        store.send(.task)
        return (store, flow)
    }

    /// Recent photos to show in + , each a square of one colour.
    func recentPhotos(_ colors: [UIColor] = [.systemTeal, .systemOrange, .systemPink, .systemIndigo])
        -> [RecentPhoto]
    {
        colors.enumerated().map { index, color in
            let image = UIGraphicsImageRenderer(size: CGSize(width: 120, height: 120)).jpegData(
                withCompressionQuality: 0.9
            ) { context in
                color.setFill()
                context.fill(CGRect(x: 0, y: 0, width: 120, height: 120))
            }
            return RecentPhoto(id: "photo-\(index)", thumbnail: image)
        }
    }

    /// Runs `body` as Shortcuts or Siri would, on the open app's records,
    /// at the app's time.
    func shortcut<T>(_ body: (Shortcuts) async throws -> T) async throws -> T {
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

    func task(_ title: String, in store: StoreOf<RootFeature>) throws -> UUID {
        try XCTUnwrap(store.workspace?.tasks.values.first { $0.title == title }?.id, "No task \(title)")
    }

    /// The assistant as the app has it, on a model that gives these
    /// `answers` in turn: what Gemma 4 E2B answered on a device, so what
    /// Lifr does with them runs as it does there.
    /// Lifr picks `skill` for each message sent without one.
    func gemma(_ answers: [JSONValue], picking skill: String = Skill.add.id) -> AssistantClient {
        let model = ScriptedRuntime(
            budget: ContextBudget(window: 32_768, reservedForAnswer: 2_048),
            steps: answers.map { ScriptedRuntime.answer($0) })
        let titles = ScriptedRuntime(steps: [ScriptedRuntime.answer("Groceries")])
        var client = AssistantClient.live(runtime: { use, _ in use == .assistant ? model : titles })
        client.pickSkill = { _, _, _, _ in skill }
        return client
    }

    func send(_ text: String, in id: Conversation.ID, of store: StoreOf<RootFeature>) {
        store.send(.conversations(.chats(.element(id: id, action: .core(.binding(.set(\.input, text)))))))
        store.send(.conversations(.chats(.element(id: id, action: .core(.sendTapped)))))
    }

    func draftTitles(_ id: Conversation.ID, in store: StoreOf<RootFeature>) -> [String] {
        store.conversations.chats[id: id]?.conversation.draft?.tasks.map(\.title) ?? []
    }
}

/// What the app last gave the widgets, for a Home Screen that shows them.
@MainActor @Observable
final class WidgetBoard {
    var glance: TodayGlance?
    /// When the Home Screen is looked at, as the widget's timeline would
    /// show it then; nil for when the app wrote it.
    var date: Date?

    nonisolated init() {}

    func show(_ glance: TodayGlance) {
        self.glance = glance
    }
}
