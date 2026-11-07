import Foundation
import LifrCore
import LifrStoreClient
import SyncKit

package typealias Syncer = SyncKit.Syncer<SyncedRecords>

extension SyncStore where Snapshot == SyncedRecords {
    /// Lifr's store as sync reads and writes it. The person's choices for
    /// records under review are saved as one journal entry, so they can be
    /// undone.
    package static func lifr(_ store: StoreClient) -> Self {
        Self(
            load: { SyncedRecords(try await store.load()) },
            importSynced: { synced in SyncedRecords(try await store.importSynced(SyncedChanges(synced)).workspace) },
            resolve: { resolved in _ = try await store.save { workspace, _ in workspace.resolveSynced(resolved) } })
    }
}

extension SyncKit.Syncer where Snapshot == SyncedRecords {
    /// Syncs `store`'s records with `cloud`.
    package init(
        cloud: Cloud, store: StoreClient, storage: LedgerStorage, now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.init(cloud: cloud, store: .lifr(store), storage: storage, now: now)
    }
}
