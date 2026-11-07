import Foundation
import JournalKit

/// One thing sync sends: a record as it now is, or a log, which never
/// changes once written.
public enum CloudItem<Snapshot: SyncSnapshot>: Hashable, Sendable {
    /// Nil when the record was removed.
    case record(RecordID<Snapshot.Record.Kind>, Snapshot.Record?)
    case log(Snapshot.Log)

    /// The item's name in the cloud.
    public var name: String {
        switch self {
        case .record(let record, _): Snapshot.cloudName(of: record)
        case .log(let log): log.cloudName
        }
    }
}

// A record is saved as `{"record": {"_0": id, "_1": value}}` and a log as the
// log itself, so a log that's an enum, such as `{"entry": {...}}`, reads as it
// did before it was a log.
extension CloudItem: Codable {
    private enum Keys: String, CodingKey {
        case record
    }

    private enum RecordKeys: String, CodingKey {
        case id = "_0"
        case value = "_1"
    }

    public init(from decoder: any Decoder) throws {
        if let container = try? decoder.container(keyedBy: Keys.self), container.contains(.record) {
            let record = try container.nestedContainer(keyedBy: RecordKeys.self, forKey: .record)
            self = .record(
                try record.decode(RecordID<Snapshot.Record.Kind>.self, forKey: .id),
                try record.decodeIfPresent(Snapshot.Record.self, forKey: .value))
        } else {
            self = .log(try Snapshot.Log(from: decoder))
        }
    }

    public func encode(to encoder: any Encoder) throws {
        switch self {
        case .record(let id, let value):
            var container = encoder.container(keyedBy: Keys.self)
            var record = container.nestedContainer(keyedBy: RecordKeys.self, forKey: .record)
            try record.encode(id, forKey: .id)
            try record.encodeIfPresent(value, forKey: .value)
        case .log(let log):
            try log.encode(to: encoder)
        }
    }
}

/// An item another device sent, with that device's name for the review.
public struct CloudChange<Snapshot: SyncSnapshot>: Hashable, Codable, Sendable {
    public var item: CloudItem<Snapshot>
    public var device: String

    public init(_ item: CloudItem<Snapshot>, from device: String) {
        self.item = item
        self.device = device
    }
}

/// The cloud as one device sees it. The cloud remembers what this device
/// fetched, so `fetch` returns only what's new.
public struct Cloud<Snapshot: SyncSnapshot>: Sendable {
    /// What other devices sent since this device last fetched. Throws
    /// `SyncError`.
    public var fetch: @Sendable () async throws -> [CloudChange<Snapshot>]
    /// Saves the items. Returns, unsaved, each item another device changed
    /// since this one last fetched it, with that device's version, so it
    /// can be merged and sent again. Throws `SyncError`.
    public var send: @Sendable (_ items: [CloudItem<Snapshot>]) async throws -> [CloudChange<Snapshot>]

    public init(
        fetch: @escaping @Sendable () async throws -> [CloudChange<Snapshot>],
        send: @escaping @Sendable (_ items: [CloudItem<Snapshot>]) async throws -> [CloudChange<Snapshot>]
    ) {
        self.fetch = fetch
        self.send = send
    }
}

/// Syncing didn't finish. The records on this device are kept as they
/// are, and what wasn't sent is sent next time.
public enum SyncError: Error, Hashable, LocalizedError, CustomStringConvertible {
    /// This build has no iCloud container, such as a build signed without
    /// a team.
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

    public var description: String {
        switch self {
        case .notSetUp:
            "This build isn't set up for iCloud, so your records stay on this device. Builds from the App "
                + "Store sync; a build you made yourself needs your Apple team and an iCloud container."
        case .noAccount:
            "Sign in to iCloud in Settings to sync your records between devices. Until then they stay on "
                + "this device, and nothing is lost."
        case .quotaExceeded:
            "Your iCloud storage is full, so changes can't be sent. Free up space or upgrade your iCloud "
                + "storage; your changes are kept here and sent once there's room."
        case .unreachable(let reason):
            "iCloud couldn't be reached (\(reason)). Your changes are kept here and sent when you're back "
                + "online."
        case .refused(let reason):
            "iCloud refused the changes (\(reason)). They're kept on this device. Try Sync Now later; if it "
                + "keeps failing, check that you're signed in to iCloud."
        case .storeFailed(let reason):
            "Your records couldn't be read or saved while syncing (\(reason)). Nothing was changed. Check "
                + "that this device has free storage, then try Sync Now."
        case .ledgerFailed(let reason):
            "What sync saw couldn't be saved (\(reason)). Your records are fine, but the next sync may ask "
                + "you to review changes again. Check that this device has free storage."
        }
    }

    public var errorDescription: String? { description }
}
