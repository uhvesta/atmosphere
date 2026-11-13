import Foundation
import JournalKit

/// What this device last agreed on with the cloud, kept between syncs: the
/// base each merge starts from, and the records held for review.
public struct SyncLedger<Snapshot: SyncSnapshot>: Hashable, Codable, Sendable {
    public typealias Record = Snapshot.Record
    public typealias RecordKey = RecordID<Record.Kind>

    /// A record that couldn't merge, kept with this device's values until
    /// the person reviews it.
    public struct Held: Hashable, Codable, Sendable {
        /// The version both devices last agreed on. It stays the same while
        /// the record is held, however often the other device changes it.
        public var base: Record?
        /// The other device's latest version.
        public var theirs: Record?
        /// The other device's name.
        public var device: String
    }

    /// Each record as the cloud last had it. A record that isn't here was
    /// never synced; a nil value was removed.
    public var bases: [RecordKey: Record?] = [:]
    public var held: [RecordKey: Held] = [:]
    /// The logs the cloud has.
    public var sent: Set<UUID> = []
    public var lastSynced: Date?
    /// What the cloud sent that the store couldn't write yet. The cloud
    /// doesn't send it again, so the next sync merges it first.
    public var unimported: [CloudChange<Snapshot>] = []

    public init() {}

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        bases = try container.decode([RecordKey: Record?].self, forKey: .bases)
        held = try container.decode([RecordKey: Held].self, forKey: .held)
        sent = try container.decode(Set<UUID>.self, forKey: .sent)
        lastSynced = try container.decodeIfPresent(Date.self, forKey: .lastSynced)
        // Ledgers kept before this was added don't have it.
        unimported = try container.decodeIfPresent([CloudChange<Snapshot>].self, forKey: .unimported) ?? []
    }

    /// Merges what other devices sent with the records, and returns what to
    /// write. Records that can't merge are held with this device's values
    /// for the fields in conflict; their other fields merge.
    public mutating func receive(_ changes: [CloudChange<Snapshot>], into snapshot: Snapshot) -> Synced<Snapshot> {
        var synced = Synced<Snapshot>()
        var latest: [RecordKey: (value: Record?, device: String)] = [:]
        var known = snapshot.logIDs
        for change in changes {
            switch change.item {
            case .record(let record, let value): latest[record] = (value, change.device)
            case .log(let log):
                sent.insert(log.id)
                if known.insert(log.id).inserted { synced.logs.append(log) }
            }
        }
        for (record, theirs) in latest {
            let base = held[record].map(\.base) ?? bases[record] ?? nil
            // One this device can't read is taken as unchanged here.
            let unreadable = snapshot.unreadable.contains(record)
            let mine = unreadable ? base : snapshot.value(of: record)
            let written: Record??
            switch Record.merge(base: base, mine: mine, theirs: theirs.value) {
            case .merged(let value):
                held[record] = nil
                bases[record] = .some(theirs.value)
                written = value != mine ? .some(value) : nil
            case .conflict(let conflict):
                held[record] = Held(base: base, theirs: theirs.value, device: theirs.device)
                written = conflict.merged != mine ? .some(conflict.merged) : nil
            }
            if let written {
                synced.records[record] = written
                if !unreadable { synced.seen[record] = .some(mine) }
            }
        }
        return synced
    }

    /// What this device changed since the cloud last had it, except records
    /// held for review.
    public func pending(_ snapshot: Snapshot) -> [CloudItem<Snapshot>] {
        var items: [CloudItem<Snapshot>] = []
        let unreadable = snapshot.unreadable
        for record in snapshot.recordIDs.union(bases.keys).sorted(by: Snapshot.sendsBefore)
        where held[record] == nil && !unreadable.contains(record) {
            let value = snapshot.value(of: record)
            if value != bases[record] ?? nil { items.append(.record(record, value)) }
        }
        items += snapshot.logs.filter { !sent.contains($0.id) }.map(CloudItem.log)
        return items
    }

    /// Notes that the cloud saved `items`.
    public mutating func saved(_ items: [CloudItem<Snapshot>]) {
        for item in items {
            switch item {
            case .record(let record, let value): bases[record] = .some(value)
            case .log(let log): sent.insert(log.id)
            }
        }
    }

    /// The held records still in conflict with this device's records, with
    /// the other device's name, in a fixed order.
    public func conflicts(_ snapshot: Snapshot) -> [ReviewItem<Record>] {
        held.sorted { Snapshot.sendsBefore($0.key, $1.key) }.compactMap { record, held in
            let result = Record.merge(base: held.base, mine: snapshot.value(of: record), theirs: held.theirs)
            guard case .conflict(let conflict) = result else { return nil }
            return ReviewItem(conflict: conflict, device: held.device)
        }
    }

    /// The records as the person chose, and stops holding them. The other
    /// device's version becomes the base, so a choice that differs from it
    /// is sent.
    public mutating func resolve(_ choices: ReviewChoices<Record>, in snapshot: Snapshot) -> [RecordKey: Record?] {
        var resolved: [RecordKey: Record?] = [:]
        let conflicts = conflicts(snapshot)
        for item in conflicts {
            let record = item.conflict.record
            guard let held = held[record] else { continue }
            let value = item.conflict.resolved(choices[record] ?? [:])
            if value != snapshot.value(of: record) { resolved[record] = .some(value) }
            bases[record] = .some(held.theirs)
            self.held[record] = nil
        }
        // Held records that merge now, such as ones edited to match.
        for (record, held) in held where !conflicts.contains(where: { $0.conflict.record == record }) {
            bases[record] = .some(held.theirs)
            self.held[record] = nil
        }
        return resolved
    }
}

/// A record that couldn't merge, for the person to review.
public struct ReviewItem<Record: Journaled>: Hashable, Sendable, Identifiable {
    public var conflict: SyncConflict<Record>
    /// The other device's name.
    public var device: String
    public var id: RecordID<Record.Kind> { conflict.record }

    public init(conflict: SyncConflict<Record>, device: String) {
        self.conflict = conflict
        self.device = device
    }
}

/// For each record, the side chosen for each field in conflict. A conflict
/// about the whole record uses the record's trash field.
public typealias ReviewChoices<Record: Journaled> = [RecordID<Record.Kind>: [Record.Field: SyncSide]]
