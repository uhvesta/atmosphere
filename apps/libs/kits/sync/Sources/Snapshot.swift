import Foundation
import JournalKit

/// Something sync sends that never changes once written, such as a journal
/// entry. Each device only adds the ones it doesn't have.
public protocol SyncLog: Hashable, Codable, Sendable {
    var id: UUID { get }
    /// Its name in the cloud, such as `journal/<id>`. No two logs or
    /// records share one.
    var cloudName: String { get }
}

/// An app's records as sync sees them at one moment: each record by ID, and
/// the logs it keeps.
public protocol SyncSnapshot: Sendable {
    associatedtype Record: Journaled
    associatedtype Log: SyncLog

    /// Every record there is now.
    var recordIDs: Set<RecordID<Record.Kind>> { get }
    /// The record as it is now; nil when there's none.
    func value(of record: RecordID<Record.Kind>) -> Record?
    /// Records that are there but this device can't read, such as ones a
    /// newer version wrote. Sync never sends them as removed.
    var unreadable: Set<RecordID<Record.Kind>> { get }
    /// The IDs of every log there is now.
    var logIDs: Set<UUID> { get }
    /// Every log, oldest first.
    var logs: [Log] { get }

    /// The record's name in the cloud, such as `task/<id>`.
    static func cloudName(of record: RecordID<Record.Kind>) -> String
    /// Whether `lhs` is sent before `rhs`, such as a folder before the
    /// notes in it, so another device never sees a note before its folder.
    static func sendsBefore(_ lhs: RecordID<Record.Kind>, _ rhs: RecordID<Record.Kind>) -> Bool
}

extension SyncSnapshot {
    /// Every log's ID, from `logs`. Override when there's a faster way.
    public var logIDs: Set<UUID> { Set(logs.map(\.id)) }

    /// None, unless the app says otherwise.
    public var unreadable: Set<RecordID<Record.Kind>> { [] }
}

/// What other devices changed, to write to this device's records.
public struct Synced<Snapshot: SyncSnapshot>: Hashable, Sendable {
    /// Each record as it now is; nil when it was removed.
    public var records: [RecordID<Snapshot.Record.Kind>: Snapshot.Record?]
    /// Logs this device didn't have, in the order they arrived.
    public var logs: [Snapshot.Log]
    /// Each record in `records` as the merge read it here. A store that
    /// finds one changed since, by an edit made while sync ran, merges that
    /// edit in rather than writing over it.
    public var seen: [RecordID<Snapshot.Record.Kind>: Snapshot.Record?]

    public init(
        records: [RecordID<Snapshot.Record.Kind>: Snapshot.Record?] = [:], logs: [Snapshot.Log] = [],
        seen: [RecordID<Snapshot.Record.Kind>: Snapshot.Record?] = [:]
    ) {
        self.records = records
        self.logs = logs
        self.seen = seen
    }

    public var isEmpty: Bool { records.isEmpty && logs.isEmpty }
}
