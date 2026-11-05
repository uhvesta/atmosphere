import ComposableArchitecture
import LifrAPI
import LifrAssistantClient
import LifrAssistantLive
import LifrCore
import LifrLiteRT
import LifrLockClient
import LifrModelsClient
import LifrModelsLive
import LifrRemindersClient
import LifrRemindersLive
import LifrRoot
import LifrSessionClient
import LifrStoreClient
import LifrStoreLive
import LifrSyncClient
import LifrSyncLive
import SwiftUI

#if canImport(UIKit)
    import UIKit
#endif
#if os(macOS)
    import LifrSessionLive
#endif

@main
struct LifrApp: App {
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
        $0.assistant = .live(runtime: { use in models.runtime(for: use) })
        $0.models = models.client
        $0.conversations = chats.conversations
        $0.skills = chats.skills
        $0.guidance = .onDisk()
        $0.attachments = .onDisk()
        $0.recorder = .live()
        $0.session = session
        $0.sync = sync
        $0.lock = .live()
    }

    /// Syncs the records with the person's other devices through iCloud,
    /// when this build has a container.
    private static let sync = SyncClient.live(
        container: LifrIdentity.iCloudContainer, store: client, device: deviceName)

    /// The models downloaded to this device, and the one that's loaded.
    nonisolated private static let models: ModelRuntimes = {
        let host = ModelHost(load: LiteRTLMEngine.load)
        Task { await host.observeSystem() }
        return ModelRuntimes(library: ModelLibrary.live(), host: host)
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
