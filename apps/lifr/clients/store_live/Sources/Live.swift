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
    /// to the store. Runs until the calling task is cancelled.
    package func reconcileAfterEachImport(as actor: Actor) async {
        let logger = Logger(subsystem: "lifr", category: "sync")
        for await _ in NotificationCenter.default.notifications(named: .NSPersistentStoreRemoteChange)
            .map({ _ in () })
        {
            do {
                let fixes = try reconcile(in: { Context(actor: .system(device: actor.device), now: Date()) })
                if !fixes.isEmpty { logger.info("Reconciled \(fixes.count) records after sync") }
            } catch {
                logger.error("Reconciling after sync failed: \(error.description, privacy: .public)")
            }
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
            setStatus: { status, id, cascade in
                try await store.save(in: context()) { (workspace, context) throws(LifrError) in
                    try workspace.setStatus(status, of: id, cascade: cascade, in: context)
                }
            },
            undo: { change in
                try await store.save(in: context()) { workspace, context in workspace.undo(change, in: context) }
            }
        )
    }
}
