import Foundation
import LifrStoreClient
import LifrSyncClient
import SyncKit
import SyncKitCloudKit

extension SyncClient {
    /// Syncs `store` through the iCloud container `container`, in the
    /// private database's "Lifr" zone, naming this device `device` to the
    /// others. With no container, such as a build signed without a team,
    /// every sync says it isn't set up.
    package static func live(container: String, store: StoreClient, device: String) -> Self {
        do {
            var client = SyncClient.syncing(
                try CloudKitSync.syncer(
                    container: container, zone: "Lifr", device: device, subsystem: "lifr", store: .lifr(store)),
                reconnected: networkReturns)
            let flush = client.flush
            let sync = client.sync
            client.flush = { await awake { await finishingInBackground(flush) } }
            client.sync = { await awake(sync) }
            return client
        } catch {
            return failing(error)
        }
    }

    /// Never syncs, and says why.
    static func failing(_ error: SyncError) -> Self {
        let state = SyncState(status: .failed(error))
        return SyncClient(
            states: { AsyncStream { $0.yield(state) } }, sync: { state }, resolve: { _ in state }, changed: {})
    }
}
