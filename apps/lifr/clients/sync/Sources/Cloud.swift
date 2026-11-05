import Foundation
import LifrCore

/// One thing sync sends: a record as it now is, or a journal entry or
/// history event, which never change once written.
package enum CloudItem: Hashable, Codable, Sendable {
    /// Nil when the record was removed.
    case record(RecordID, RecordValue?)
    case entry(JournalEntry)
    case event(HistoryEvent)

    /// The item's name in the cloud, such as `task/<id>`. Each record kind,
    /// and entries and events, have their own prefix, so names never clash.
    package var name: String {
        switch self {
        case .record(let record, _): Self.name(of: record)
        case .entry(let entry): "journal/\(entry.id.uuidString)"
        case .event(let event): "history/\(event.id.uuidString)"
        }
    }

    package static func name(of record: RecordID) -> String {
        record.kind == .settings ? "settings" : "\(record.kind.rawValue)/\(record.id.uuidString)"
    }
}

/// An item another device sent, with that device's name for the review.
package struct CloudChange: Hashable, Codable, Sendable {
    package var item: CloudItem
    package var device: String

    package init(_ item: CloudItem, from device: String) {
        self.item = item
        self.device = device
    }
}

/// The cloud as one device sees it. The cloud remembers what this device
/// fetched, so `fetch` returns only what's new.
package struct Cloud: Sendable {
    /// What other devices sent since this device last fetched. Throws
    /// `SyncError`.
    package var fetch: @Sendable () async throws -> [CloudChange]
    /// Saves the items. Returns, unsaved, each item another device changed
    /// since this one last fetched it, with that device's version, so it
    /// can be merged and sent again. Throws `SyncError`.
    package var send: @Sendable (_ items: [CloudItem]) async throws -> [CloudChange]

    package init(
        fetch: @escaping @Sendable () async throws -> [CloudChange],
        send: @escaping @Sendable (_ items: [CloudItem]) async throws -> [CloudChange]
    ) {
        self.fetch = fetch
        self.send = send
    }
}

/// Syncing didn't finish. The records on this device are kept as they
/// are, and what wasn't sent is sent next time.
package enum SyncError: Error, Hashable, LocalizedError, CustomStringConvertible {
    /// This build of Lifr has no iCloud container, such as a build signed
    /// without a team.
    case notSetUp
    /// No iCloud account is signed in on this device, or iCloud Drive is off.
    case noAccount
    /// The iCloud storage is full.
    case quotaExceeded
    /// The network or iCloud didn't answer.
    case unreachable(String)
    /// iCloud refused the change for another reason.
    case refused(String)
    /// Reading or writing this device's records failed.
    case storeFailed(String)
    /// Saving what sync last saw failed.
    case ledgerFailed(String)

    package var description: String {
        switch self {
        case .notSetUp:
            "This build of Lifr isn't set up for iCloud, so your records stay on this device. Builds from "
                + "the App Store sync; a build you made yourself needs your Apple team and an iCloud container "
                + "set in user.bazelrc."
        case .noAccount:
            "Sign in to iCloud in Settings to sync your records between devices. Until then they stay on "
                + "this device, and nothing is lost."
        case .quotaExceeded:
            "Your iCloud storage is full, so Lifr can't send changes. Free up space or upgrade your iCloud "
                + "storage; your changes are kept here and sent once there's room."
        case .unreachable(let reason):
            "Lifr couldn't reach iCloud (\(reason)). Your changes are kept here and sent when you're back "
                + "online."
        case .refused(let reason):
            "iCloud refused the changes (\(reason)). They're kept on this device. Try Sync Now later; if it "
                + "keeps failing, check that you're signed in to iCloud."
        case .storeFailed(let reason):
            "Lifr couldn't read or save its records while syncing (\(reason)). Nothing was changed. Check "
                + "that this device has free storage, then try Sync Now."
        case .ledgerFailed(let reason):
            "Lifr couldn't save what it synced (\(reason)). Your records are fine, but the next sync may "
                + "ask you to review changes again. Check that this device has free storage."
        }
    }

    package var errorDescription: String? { description }
}
