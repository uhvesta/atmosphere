import CloudKit
import Foundation
import SyncKit

/// Sync through CloudKit: a CKSyncEngine in the private database, each item
/// encrypted end to end, and what sync last saw kept in files only the app
/// can read.
public enum CloudKitSync {
    /// A syncer for `store` through the iCloud container `container`, keeping
    /// the records in `zone` and naming this device `device` to the others.
    /// `subsystem` names the app in the log.
    ///
    /// Throws `SyncError.notSetUp` when `container` is empty, such as in a
    /// build signed without a team, and `SyncError.ledgerFailed` when the
    /// sync folder can't be made.
    public static func syncer<Snapshot: SyncSnapshot>(
        container: String, zone: String, device: String, subsystem: String, store: SyncStore<Snapshot>
    ) throws(SyncError) -> Syncer<Snapshot> {
        guard !container.isEmpty else { throw .notSetUp }
        let files = try SyncFiles.onDisk()
        let cloud = CloudKitCloud<Snapshot>(
            container: CKContainer(identifier: container), zone: zone, device: device, storage: files.engine,
            subsystem: subsystem)
        return Syncer(cloud: cloud.cloud, store: store, storage: files.ledger())
    }
}
