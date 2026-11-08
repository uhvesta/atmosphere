import AppIntents
import ComposableArchitecture
import InferenceKit
import InferenceKitLiteRT
import InferenceKitLive
import InferenceKitSpeech
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
import LifrStoreClient
import LifrStoreLive
import LifrSyncClient
import LifrSyncLive
import SwiftUI

#if canImport(InferenceKitLlama)
    import InferenceKitLlama
#endif
#if canImport(InferenceKitMLX)
    import InferenceKitMLX
#endif

#if canImport(UIKit)
    import UIKit
#endif
#if os(macOS)
    import LifrSessionLive
#endif

@main
struct LifrApp: App {
    init() {
        let shortcuts = Storage.shortcuts
        AppDependencyManager.shared.add { shortcuts }
    }

    var body: some Scene {
        #if os(macOS)
            window
                // Room for the sidebar, a list and a task side by side.
                .defaultSize(width: 1180, height: 760)
                .commands { RootCommands(store: Storage.root) }
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
                // Woken for downloads the system finished while Lifr was away.
                .backgroundTask(.urlSession(ModelLibrary.backgroundSession)) {
                    _ = Storage.models
                    await ModelLibrary.caughtUp()
                }
        #endif
    }

    private var window: some Scene {
        WindowGroup {
            switch Storage.opened {
            case .success(let store):
                RootView(store: Storage.root)
                    .task {
                        if ProcessInfo.processInfo.arguments.contains(Storage.sampleDataArgument) {
                            try? await store.insert(.sample(now: Date(), timeZone: .current))
                        }
                    }
            case .failure(let error):
                ContentUnavailableView(
                    "Lifr can't open its data", systemImage: "exclamationmark.triangle",
                    description: Text(error.description))
            }
        }
    }
}

@MainActor
private enum Storage {
    /// Launch with this argument to add sample records, for trying out the
    /// app without typing them in.
    static let sampleDataArgument = "-LifrSampleData"

    static let opened = Result { () throws(LiveStoreError) in try LiveStore(modelContainer: LiveStore.onDisk()) }

    static let actor = Actor.user(device: deviceName, surface: surface)

    /// One store for every screen, so each change shows everywhere.
    static let root = Store(initialState: RootFeature.State(platform: platform)) {
        RootFeature()
    } withDependencies: {
        $0.store = client
        $0.reminders = .live()
        $0.assistant = .live(runtime: { use, choice in models.runtime(for: use, choice: choice) })
        $0.relatedWords = .live(
            selected: { models.selection.model(for: .search) }, runtime: { models.runtime(for: .search) })
        $0.models = models.client
        $0.conversations = chats.conversations
        $0.skills = chats.skills
        $0.guidance = .onDisk()
        $0.attachments = .onDisk()
        $0.recorder = .live()
        $0.speech = speech
        $0.dictation = .live()
        $0.photoLibrary = .live()
        $0.fileReader = .live
        $0.session = session
        $0.sync = sync
        $0.lock = .live()
        $0.glance = .live(file: GlanceFile.shared)
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
            output: SpeakerOutput(), engine: { model, _ in SupertonicEngine(model: model) })
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
            store: store, locksApp: { Shared(.locksApp).wrappedValue }, now: { Date() }, timeZone: .current)
    }()

    static let client: StoreClient =
        if case .success(let store) = opened { .live(store, actor: actor) } else { StoreClient() }

    private static var platform: RootFeature.Platform {
        #if os(macOS)
            .mac
        #else
            .phone
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
