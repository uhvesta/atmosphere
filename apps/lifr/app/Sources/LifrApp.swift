import AppIntents
import ComposableArchitecture
import InferenceKit
import InferenceKitLiteRT
import InferenceKitLive
import InferenceKitSpeech
import InferenceKitVoices
import LifrAPI
import LifrAssistantClient
import LifrAssistantLive
import LifrCore
import LifrGlance
import LifrGlanceClient
import LifrLockClient
import LifrModelsClient
import LifrModelsLive
import LifrRemindersClient
import LifrRemindersLive
import LifrRoot
import LifrSearchClient
import LifrSearchLive
import LifrSessionClient
import LifrShortcuts
import LifrSpotlightClient
import LifrStoreClient
import LifrStoreLive
import LifrSyncClient
import LifrSyncLive
import LifrWatchLink
import SwiftUI
import UserNotifications

#if canImport(InferenceKitLlama)
    import InferenceKitLlama
#endif
#if canImport(InferenceKitMLX)
    import InferenceKitMLX
#endif

#if canImport(UIKit)
    import UIKit
#endif
#if os(iOS)
    import BackgroundTasks
    import OSLog
#endif
#if os(macOS)
    import LifrSessionLive
#endif

@main
struct LifrApp: App {
    @Environment(\.scenePhase) private var scenePhase

    init() {
        let shortcuts = Storage.shortcuts
        AppDependencyManager.shared.add { shortcuts }
        UNUserNotificationCenter.current().delegate = Storage.notifications
        #if os(macOS)
            // The Mac keeps syncing and scheduling reminders from the menu
            // bar with every window closed, so the work outlives windows.
            if case .success = Storage.opened {
                Storage.start()
                Storage.capture.start()
            }
            NSApplication.shared.servicesProvider = Storage.services
        #else
            if case .success = Storage.opened {
                WatchSession.shared.activate { await Storage.saveFromWatch($0) }
            }
        #endif
    }

    var body: some Scene {
        #if os(macOS)
            window
                // Room for the sidebar, a list and a task side by side.
                .defaultSize(width: 1180, height: 760)
                .commands {
                    // View › Show Sidebar, which the split view doesn't add.
                    SidebarCommands()
                    RootCommands(store: Storage.root)
                }
            WindowGroup(id: WindowPlace.sceneID, for: WindowPlace.self) { $place in
                if case .success = Storage.opened {
                    OtherWindow(main: Storage.root, place: $place, store: Storage.window(at:))
                }
            }
            .defaultSize(width: 1180, height: 760)
            Settings {
                MacSettingsView(store: Storage.root)
            }
            MenuBarExtra {
                MenuBarContent(store: Storage.root)
            } label: {
                MenuBarLabel(store: Storage.root)
            }
        #else
            window
                .commands { RootCommands(store: Storage.root) }
                // Woken for downloads the system finished while Lifr was away.
                .backgroundTask(.urlSession(ModelLibrary.backgroundSession)) {
                    _ = Storage.models
                    await ModelLibrary.caughtUp()
                }
                // Woken now and then while away, to sync, so reminders and
                // widgets follow what other devices changed.
                .backgroundTask(.appRefresh(Storage.refreshID)) {
                    await Storage.refresh()
                }
                // Asked for on leaving, once the refresh above is registered:
                // asking before then, as from init, aborts the app.
                .onChange(of: scenePhase) { _, phase in
                    if phase == .background { Storage.scheduleRefresh() }
                }
            // The iPad's other windows; the iPhone opens none.
            WindowGroup(id: WindowPlace.sceneID, for: WindowPlace.self) { $place in
                if case .success = Storage.opened {
                    OtherWindow(main: Storage.root, place: $place, store: Storage.window(at:))
                }
            }
        #endif
    }

    private var window: some Scene {
        WindowGroup(id: MenuBarContent.windowID) {
            OpenedRoot()
        }
    }
}

/// The main window: the records, or why Lifr couldn't open them.
private struct OpenedRoot: View {
    var body: some View {
        switch Storage.opening.result {
        case .success(let store):
            RootView(store: Storage.root)
                .task {
                    if ProcessInfo.processInfo.arguments.contains(Storage.sampleDataArgument) {
                        try? await store.insert(.sample(now: Date(), timeZone: .current))
                    }
                }
        case .failure(let error):
            OpenFailureView(store: Storage.root, reason: error.description) {
                #if os(macOS)
                    // The menu bar and menus were made without the records,
                    // so only a fresh start picks them up.
                    Button("Reopen Lifr") { Storage.relaunch() }
                #else
                    Button("Try Again") { Storage.opening.retry() }
                #endif
            }
        }
    }
}

/// Opens the records, and again when the person asks after a failure.
@MainActor @Observable
private final class Opening {
    private(set) var result = Opening.open()

    func retry() {
        guard case .failure = result else { return }
        result = Self.open()
    }

    private static func open() -> Result<LiveStore, LiveStoreError> {
        Result { () throws(LiveStoreError) in try LiveStore(modelContainer: LiveStore.onDisk()) }
    }
}

@MainActor
private enum Storage {
    #if os(iOS)
        /// The background refresh, listed in Info.plist.
        nonisolated static let refreshID = (Bundle.main.bundleIdentifier ?? "lifr") + ".refresh"

        /// Asks the system to wake Lifr to refresh in a while; it picks when.
        static func scheduleRefresh() {
            let request = BGAppRefreshTaskRequest(identifier: refreshID)
            request.earliestBeginDate = Date(timeIntervalSinceNow: 15 * 60)
            do {
                try BGTaskScheduler.shared.submit(request)
            } catch {
                // Background App Refresh off, or the simulator: Lifr catches
                // up when it opens instead.
                Logger(subsystem: "lifr", category: "refresh")
                    .notice("\(String(describing: error), privacy: .public)")
            }
        }

        /// Syncs, then reschedules reminders and refreshes widgets, and
        /// asks for the next refresh.
        static func refresh() async {
            scheduleRefresh()
            guard case .success = opened else { return }
            await root.send(.refreshedInBackground).finish()
        }
    #endif

    /// Launch with this argument to add sample records, for trying out the
    /// app without typing them in.
    static let sampleDataArgument = "-LifrSampleData"

    static let opening = Opening()

    /// The records, once opened. Everything made from them is made after.
    static var opened: Result<LiveStore, LiveStoreError> { opening.result }

    #if os(macOS)
        /// Starts a new Lifr and quits this one.
        static func relaunch() {
            let configuration = NSWorkspace.OpenConfiguration()
            configuration.createsNewApplicationInstance = true
            NSWorkspace.shared.openApplication(at: Bundle.main.bundleURL, configuration: configuration) { _, _ in
                Task { @MainActor in NSApplication.shared.terminate(nil) }
            }
        }
    #endif

    static let actor = Actor.user(device: deviceName, surface: surface)

    /// Kept for as long as Lifr runs: the notification center holds its
    /// delegate weakly.
    static let notifications = ReminderNotifications(
        open: { root.send(.reminderOpened($0)) },
        changeStatus: { root.send(.reminderStatusRequested($0)) },
        review: { root.send(.syncReviewTapped) },
        today: { root.send(.linkOpened(.today)) },
        shortcuts: { shortcuts })

    #if os(macOS)
        /// Kept for as long as Lifr runs, for the Services menu's Add to Lifr.
        static let services = AddToLifrService { root.send(.textSent($0)) }

        /// The root store's long-lived work, kept for as long as Lifr runs.
        private static var running: Task<Void, Never>?

        static func start() {
            guard running == nil else { return }
            running = Task { await root.send(.task).finish() }
            let changes = [
                (NSApplication.didBecomeActiveNotification, true), (NSApplication.didResignActiveNotification, false),
            ]
            activation = changes.map { name, active in
                Task {
                    for await _ in NotificationCenter.default.notifications(named: name) {
                        root.send(.appActivated(active))
                    }
                }
            }
        }

        /// Tells the root store when Lifr comes to the front or leaves it, for
        /// as long as Lifr runs.
        private static var activation: [Task<Void, Never>] = []
    #endif

    /// The first window's store, which also syncs, schedules reminders and
    /// serves connected apps for as long as Lifr runs.
    static let root = Store(initialState: RootFeature.State(platform: platform)) {
        RootFeature()
    } withDependencies: {
        dependencies(&$0)
    }

    #if os(macOS)
        /// Quick Add over any app, from the keys picked in Settings.
        static let capture = CapturePanel(store: root)
    #endif

    /// Another Mac or iPad window's own store, which shares the records.
    static func window(at place: Place) -> StoreOf<RootFeature> {
        Store(initialState: RootFeature.State(window: place, platform: platform)) {
            RootFeature()
        } withDependencies: {
            dependencies(&$0)
        }
    }

    private static func dependencies(_ values: inout DependencyValues) {
        values.store = client
        values.thisDevice = deviceName
        values.reminders = .live()
        values.assistant = .live(runtime: { use, choice in models.runtime(for: use, choice: choice) })
        values.relatedWords = .live(
            selected: { models.selection.model(for: .search) }, runtime: { models.runtime(for: .search) })
        values.models = models.client
        values.conversations = chats.conversations
        values.skills = chats.skills
        values.guidance = .onDisk()
        values.attachments = .onDisk()
        values.recorder = .live()
        values.speech = speech
        values.session = session
        values.sync = sync
        values.lock = .live()
        var glance = GlanceClient.live(file: GlanceFile.shared)
        #if os(iOS)
            // The Watch shows the same Today.
            let write = glance.write
            glance.write = { today in
                await write(today)
                WatchSession.shared.send(today)
            }
        #endif
        values.glance = glance
        // Sample records stay out of the device's search.
        if !ProcessInfo.processInfo.arguments.contains(sampleDataArgument) { values.spotlight = .live() }
    }

    /// Syncs the records with the person's other devices through iCloud,
    /// when this build has a container.
    private static let sync = SyncClient.live(
        container: LifrIdentity.iCloudContainer, store: client, device: deviceName)

    /// Reading replies aloud in the voice picked: the system's until a
    /// voice model is downloaded and picked. The pick is kept with the
    /// models.
    private static let speech: SpeechClient = {
        let selection = VoiceSelection(file: ModelLibrary.defaultRoot.appending(path: "voices.json"))
        let speaker = Speaker(
            installed: { models.library.installed() }, selection: selection, system: SystemVoice(),
            output: SpeakerOutput(), engine: VoiceEngines.engine)
        return .picks(
            selection,
            speak: { text, language throws(SpeechError) in try await speaker.speak(text, language: language) },
            stop: { await speaker.stop() })
    }()

    /// The models downloaded to this device, and the one that's loaded.
    nonisolated static let models: ModelRuntimes<ModelUse> = {
        var loaders: [RuntimeKind: ModelHost.Loader] = [
            .liteRT: { model, backend async throws(ModelRuntimeError) in
                try await LiteRTLMEngine.load(model, on: backend)
            }
        ]
        var measures: [RuntimeKind: Bench.Measure] = [.liteRT: LiteRTLMEngine.measure]
        // llama.cpp has no build for the Simulator, whose models run on
        // the Mac. A closure that throws a type crashes the compiler when
        // it's stored straight into a dictionary's subscript, so each gets
        // its type first.
        #if canImport(InferenceKitLlama)
            let llama: ModelHost.Loader = { model, backend async throws(ModelRuntimeError) in
                try await LlamaEngine.load(model, on: backend)
            }
            let measure: Bench.Measure = LlamaEngine.measure
            loaders[.llama] = llama
            measures[.llama] = measure
        #endif
        #if canImport(InferenceKitMLX)
            let mlx: ModelHost.Loader = { model, backend async throws(ModelRuntimeError) in
                try await MLXEngine.load(model, on: backend)
            }
            let measureMLX: Bench.Measure = MLXEngine.measure
            loaders[.mlx] = mlx
            measures[.mlx] = measureMLX
        #endif
        let host = ModelHost(
            load: ModelHost.loading(with: loaders), answersInBackground: { Shared(.answersInBackground).wrappedValue })
        Task { await host.observeSystem() }
        let library = ModelLibrary.live()
        library.resumeUnfinished()
        let bench = Bench(measure: Bench.measuring(with: measures), load: ModelHost.loading(with: loaders))
        return ModelRuntimes(library: library, host: host, bench: bench)
    }()

    /// On the Mac, the local API that `lifr` connects to. The iPhone has no
    /// CLI, so no app can connect there.
    private static let session: SessionClient = {
        #if os(macOS)
            let server = SessionServer.live(store: client.telling(sync), device: deviceName)
            Task { await server.start() }
            return server.client
        #else
            return InMemorySession().client
        #endif
    }()

    /// Conversations with the assistant and the person's skills: each
    /// conversation on this device until the person syncs it, and skills
    /// with the records.
    private static let chats: (conversations: ConversationsClient, skills: SkillsClient) = {
        do {
            let store = try opened.get()
            let chats = try ConversationStore.onDisk(movingFrom: store.modelContainer)
            return (.live(chats), .live(chats))
        } catch {
            return (.failing(.openFailed(error)), .failing(.openFailed(error)))
        }
    }()

    /// What Shortcuts and Siri do in Lifr. Their changes show on the open
    /// screens as they happen, sync like any other, and are named after
    /// Shortcuts in Activity.
    static let shortcuts: Shortcuts = {
        let store: StoreClient =
            if case .success(let live) = opened {
                .live(live, actor: .user(device: deviceName, surface: .shortcuts), announces: true).telling(sync)
            } else {
                StoreClient()
            }
        return Shortcuts(
            store: store, locksApp: { Shared(.locksApp).wrappedValue }, now: { Date() }, timeZone: .current,
            defaultReminder: { Shared(.defaultReminder).wrappedValue.trigger(dueTime: $0) })
    }()

    #if os(iOS)
        /// What the Watch asks for, saved as a change made on this iPhone.
        /// Like Shortcuts' changes, it shows on the open screens, and the
        /// Today sent back to the Watch has it.
        private static let watch: Shortcuts? =
            if case .success(let live) = opened {
                Shortcuts(
                    store: .live(live, actor: actor, announces: true).telling(sync),
                    locksApp: { Shared(.locksApp).wrappedValue }, now: { Date() }, timeZone: .current,
                    defaultReminder: { Shared(.defaultReminder).wrappedValue.trigger(dueTime: $0) })
            } else {
                nil
            }

        /// Saves a change made on the Watch, or says in a notification why
        /// it wasn't, since Lifr may not be showing.
        static func saveFromWatch(_ request: WatchRequest) async {
            guard let watch else { return }
            do {
                switch request {
                case .setStatus(let id, let status):
                    _ = try await watch.setStatus(TaskStatus(status), of: id)
                case .add(let title):
                    _ = try await watch.add(title, avenue: nil, due: .none)
                case .snooze(let id):
                    try await watch.snooze(id, until: Date() + ReminderAction.snoozeInterval)
                }
            } catch {
                let content = UNMutableNotificationContent()
                content.title = String(localized: "Couldn't save the change made on the Watch")
                content.body = String(
                    localized: "\(error.localizedDescription) Open Lifr on the iPhone and make it there.")
                let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
                try? await UNUserNotificationCenter.current().add(request)
            }
        }
    #endif

    /// On the Mac each save reaches every window's store, so a change in
    /// one window shows in the others.
    static let client: StoreClient =
        if case .success(let store) = opened {
            .live(store, actor: actor, announces: platform.hasSidebar)
        } else {
            StoreClient()
        }

    private static var platform: RootFeature.Platform {
        #if os(macOS)
            .mac
        #else
            UIDevice.current.userInterfaceIdiom == .pad ? .pad : .phone
        #endif
    }

    private static var surface: Actor.Surface {
        #if os(macOS)
            .mac
        #else
            UIDevice.current.userInterfaceIdiom == .pad ? .iPad : .iPhone
        #endif
    }

    private static var deviceName: String {
        #if os(macOS)
            Host.current().localizedName ?? "Mac"
        #else
            UIDevice.current.name
        #endif
    }
}

/// Shows a reminder while Lifr is open, opens its task when the person
/// taps the notification, and carries out the button they long-pressed for.
private final class ReminderNotifications: NSObject, UNUserNotificationCenterDelegate, Sendable {
    private let open: @MainActor @Sendable (UUID) -> Void
    private let changeStatus: @MainActor @Sendable (UUID) -> Void
    private let review: @MainActor @Sendable () -> Void
    private let today: @MainActor @Sendable () -> Void
    private let shortcuts: @MainActor @Sendable () -> Shortcuts

    init(
        open: @escaping @MainActor @Sendable (UUID) -> Void,
        changeStatus: @escaping @MainActor @Sendable (UUID) -> Void,
        review: @escaping @MainActor @Sendable () -> Void,
        today: @escaping @MainActor @Sendable () -> Void,
        shortcuts: @escaping @MainActor @Sendable () -> Shortcuts
    ) {
        self.open = open
        self.changeStatus = changeStatus
        self.review = review
        self.today = today
        self.shortcuts = shortcuts
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .list, .sound]
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        if response.notification.request.content.userInfo[RemindersClient.reviewKey] != nil {
            await review()
            return
        }
        if response.notification.request.content.userInfo[RemindersClient.summaryKey] != nil {
            await today()
            return
        }
        guard let raw = response.notification.request.content.userInfo[RemindersClient.taskIDKey] as? String,
            let id = UUID(uuidString: raw)
        else { return }
        // The saved change reaches the open app, which schedules the
        // reminders and badge again.
        switch ReminderAction(rawValue: response.actionIdentifier) {
        case .done:
            do {
                _ = try await shortcuts().finish(id)
            } catch {
                await tell(String(localized: "Couldn't mark the task done"), error)
            }
        case .snooze:
            do {
                try await shortcuts().snooze(id, until: Date() + ReminderAction.snoozeInterval)
            } catch {
                await tell(String(localized: "Couldn't snooze the reminder"), error)
            }
        case .changeStatus:
            await changeStatus(id)
        case nil:
            await open(id)
        }
    }

    /// A button that failed in the background says so in a notification,
    /// since nothing else is showing.
    private func tell(_ title: String, _ error: any Error) async {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = error.localizedDescription
        let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        try? await UNUserNotificationCenter.current().add(request)
    }
}

#if os(macOS)
    /// The Services menu's Add to Lifr: hands the text selected in another
    /// app to `add`, and brings Lifr forward.
    @MainActor
    private final class AddToLifrService: NSObject {
        private let add: @MainActor (String) -> Void

        init(add: @escaping @MainActor (String) -> Void) {
            self.add = add
        }

        @objc func addToLifr(
            _ pasteboard: NSPasteboard, userData: String?, error: AutoreleasingUnsafeMutablePointer<NSString?>
        ) {
            // Text, or a link dropped on the Dock icon.
            let text = pasteboard.string(forType: .string) ?? pasteboard.string(forType: .URL)
            guard let text, !text.isEmpty else {
                let message = String(localized: "Select some text to add to Lifr, then choose Add to Lifr again.")
                error.pointee = message as NSString
                return
            }
            NSApplication.shared.activate()
            add(text)
        }
    }
#endif

#if os(iOS)
    extension TaskStatus {
        /// The status picked on the Watch.
        fileprivate init(_ status: WatchStatus) {
            switch status {
            case .todo: self = .todo
            case .inProgress: self = .inProgress
            case .procrastinated: self = .procrastinated
            case .done: self = .done
            case .wontDo: self = .wontDo
            }
        }
    }
#endif
