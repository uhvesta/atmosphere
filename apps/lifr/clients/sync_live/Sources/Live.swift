import CloudKit
import Foundation
import LifrStoreClient
import LifrSyncClient

extension SyncClient {
    /// Syncs `store` through the iCloud container `container`, naming this
    /// device `device` to the others. With no container, such as a build
    /// signed without a team, every sync says it isn't set up.
    package static func live(container: String, store: StoreClient, device: String) -> Self {
        guard !container.isEmpty else { return failing(.notSetUp) }
        let files: SyncFiles
        do {
            files = try SyncFiles.onDisk()
        } catch {
            return failing(error)
        }
        let cloud = CloudKitCloud(container: CKContainer(identifier: container), device: device, storage: files.engine)
        return .syncing(Syncer(cloud: cloud.cloud, store: store, storage: files.ledger))
    }

    /// Never syncs, and says why.
    static func failing(_ error: SyncError) -> Self {
        let state = SyncState(status: .failed(error))
        return SyncClient(
            states: { AsyncStream { $0.yield(state) } }, sync: { state }, resolve: { _ in state }, changed: {})
    }
}
