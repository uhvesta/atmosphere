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
            return .syncing(
                try CloudKitSync.syncer(
                    container: container, zone: "Lifr", device: device, subsystem: "lifr", store: .lifr(store)))
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
