import ComposableArchitecture
import CoreData
import Foundation
import LifrCore
import LifrStoreClient
import OSLog
import SwiftData

extension LiveStore {
    /// The schema for every Lifr model.
    package static let schema = Schema(models)

    /// The app's container, on disk as Lifr.store in Application Support:
    /// the app's own container when it's sandboxed. It syncs through
    /// CloudKit's private database when the app has the iCloud entitlement,
    /// using its first container; without it, it stays on this device.
    package static func onDisk() throws(LiveStoreError) -> ModelContainer {
        try container(ModelConfiguration("Lifr", schema: schema, cloudKitDatabase: .automatic))
    }

    /// A container that is never written to disk or synced, for tests.
    package static func inMemory() throws(LiveStoreError) -> ModelContainer {
        try container(ModelConfiguration(schema: schema, isStoredInMemoryOnly: true, cloudKitDatabase: .none))
    }

    private static func container(_ configuration: ModelConfiguration) throws(LiveStoreError) -> ModelContainer {
        do {
            return try ModelContainer(for: schema, configurations: configuration)
        } catch {
            throw .openFailed(error)
        }
    }

    /// Reconciles after each change that another process or CloudKit makes
    /// to the store, and sends the records with what it fixed. Ends when the
    /// calling task is cancelled.
    package nonisolated func updates(as actor: Actor) -> AsyncStream<Update> {
        AsyncStream { continuation in
            let task = Task {
                let logger = Logger(subsystem: "lifr", category: "sync")
                for await _ in NotificationCenter.default.notifications(named: .NSPersistentStoreRemoteChange)
                    .map({ _ in () })
                {
                    do {
                        let fixes = try await reconcile(in: {
                            Context(actor: .system(device: actor.device, surface: actor.surface), now: Date())
                        })
                        if !fixes.isEmpty { logger.info("Reconciled \(fixes.count) records after sync") }
                        continuation.yield(Update(workspace: try await load(), fixes: fixes))
                    } catch {
                        logger.error("Reconciling after sync failed: \(String(describing: error), privacy: .public)")
                    }
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}

extension StoreClient {
    /// Keeps the records in `store`. Changes use the `date`, `uuid` and
    /// `withRandomNumberGenerator` dependencies, as `inMemory` does.
    package static func live(_ store: LiveStore, actor: Actor) -> Self {
        @Sendable func context() -> @Sendable () -> Context {
            @Dependency(\.date.now) var now
            @Dependency(\.uuid) var uuid
            @Dependency(\.withRandomNumberGenerator) var random
            return {
                Context(
                    actor: actor, now: now, newID: { uuid() },
                    newCode: { random { TaskCode.random(using: &$0) } })
            }
        }

        return StoreClient(
            load: { try await store.load() },
            save: { operation in try await store.save(in: context(), operation) },
            updates: { store.updates(as: actor) }
        )
    }
}
