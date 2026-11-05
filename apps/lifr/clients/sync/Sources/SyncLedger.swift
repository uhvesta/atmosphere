import Foundation
import LifrCore
import LifrStoreClient

/// What this device last agreed on with the cloud, kept between syncs: the
/// base each merge starts from, and the records held for review.
package struct SyncLedger: Hashable, Codable, Sendable {
    /// A record that couldn't merge, kept with this device's values until
    /// the person reviews it.
    package struct Held: Hashable, Codable, Sendable {
        /// The version both devices last agreed on. It stays the same while
        /// the record is held, however often the other device changes it.
        package var base: RecordValue?
        /// The other device's latest version.
        package var theirs: RecordValue?
        /// The other device's name.
        package var device: String
    }

    /// Each record as the cloud last had it. A record that isn't here was
    /// never synced; a nil value was removed.
    package var bases: [RecordID: RecordValue?] = [:]
    package var held: [RecordID: Held] = [:]
    /// The journal entries and history events the cloud has.
    package var sent: Set<UUID> = []
    package var lastSynced: Date?

    package init() {}

    /// Merges what other devices sent with the records, and returns what to
    /// write. Records that can't merge are held with this device's values
    /// for the fields in conflict; their other fields merge.
    package mutating func receive(_ changes: [CloudChange], into workspace: Workspace) -> SyncedChanges {
        var synced = SyncedChanges()
        var latest: [RecordID: (value: RecordValue?, device: String)] = [:]
        for change in changes {
            switch change.item {
            case .record(let record, let value): latest[record] = (value, change.device)
            case .entry(let entry):
                sent.insert(entry.id)
                if !workspace.journal.contains(where: { $0.id == entry.id }) { synced.journal.append(entry) }
            case .event(let event):
                sent.insert(event.id)
                if !workspace.history.contains(where: { $0.id == event.id }) { synced.history.append(event) }
            }
        }
        for (record, theirs) in latest {
            let mine = workspace.value(of: record)
            let base = held[record].map(\.base) ?? bases[record] ?? nil
            switch RecordValue.merge(base: base, mine: mine, theirs: theirs.value) {
            case .merged(let value):
                held[record] = nil
                bases[record] = .some(theirs.value)
                if value != mine { synced.records[record] = .some(value) }
            case .conflict(let conflict):
                held[record] = Held(base: base, theirs: theirs.value, device: theirs.device)
                if conflict.merged != mine { synced.records[record] = .some(conflict.merged) }
            }
        }
        return synced
    }

    /// What this device changed since the cloud last had it, except records
    /// held for review.
    package func pending(_ workspace: Workspace) -> [CloudItem] {
        var items: [CloudItem] = []
        for record in Self.records(in: workspace).union(bases.keys).sorted(by: Self.order)
        where held[record] == nil {
            let value = workspace.value(of: record)
            if value != bases[record] ?? nil { items.append(.record(record, value)) }
        }
        items += workspace.journal.filter { !sent.contains($0.id) }.map(CloudItem.entry)
        items += workspace.history.filter { !sent.contains($0.id) }.map(CloudItem.event)
        return items
    }

    /// Notes that the cloud saved `items`.
    package mutating func saved(_ items: [CloudItem]) {
        for item in items {
            switch item {
            case .record(let record, let value): bases[record] = .some(value)
            case .entry(let entry): sent.insert(entry.id)
            case .event(let event): sent.insert(event.id)
            }
        }
    }

    /// The held records still in conflict with this device's records, with
    /// the other device's name, in a fixed order.
    package func conflicts(_ workspace: Workspace) -> [Review.Item] {
        held.sorted { Self.order($0.key, $1.key) }.compactMap { record, held in
            let result = RecordValue.merge(base: held.base, mine: workspace.value(of: record), theirs: held.theirs)
            guard case .conflict(let conflict) = result else { return nil }
            return Review.Item(conflict: conflict, device: held.device)
        }
    }

    /// The records as the person chose, and stops holding them. The other
    /// device's version becomes the base, so a choice that differs from it
    /// is sent.
    package mutating func resolve(_ choices: Review.Choices, in workspace: Workspace) -> [RecordID: RecordValue?] {
        var resolved: [RecordID: RecordValue?] = [:]
        for item in conflicts(workspace) {
            let record = item.conflict.record
            guard let held = held[record] else { continue }
            let value = item.conflict.resolved(choices[record] ?? [:])
            if value != workspace.value(of: record) { resolved[record] = .some(value) }
            bases[record] = .some(held.theirs)
            self.held[record] = nil
        }
        // Held records that merge now, such as ones edited to match.
        for (record, held) in held where !conflicts(workspace).contains(where: { $0.conflict.record == record }) {
            bases[record] = .some(held.theirs)
            self.held[record] = nil
        }
        return resolved
    }

    /// Every record in the workspace.
    static func records(in workspace: Workspace) -> Set<RecordID> {
        var records: Set<RecordID> = [.settings]
        records.formUnion(workspace.avenues.keys.map { RecordID(.avenue, $0) })
        records.formUnion(workspace.projects.keys.map { RecordID(.project, $0) })
        records.formUnion(workspace.tasks.keys.map { RecordID(.task, $0) })
        records.formUnion(workspace.labels.keys.map { RecordID(.label, $0) })
        records.formUnion(workspace.comments.keys.map { RecordID(.comment, $0) })
        records.formUnion(workspace.reminders.keys.map { RecordID(.reminder, $0) })
        return records
    }

    /// Records others refer to first, then by ID.
    static func order(_ lhs: RecordID, _ rhs: RecordID) -> Bool {
        let kinds = RecordKind.allCases
        let left = kinds.firstIndex(of: lhs.kind) ?? 0
        let right = kinds.firstIndex(of: rhs.kind) ?? 0
        return (left, lhs.id.uuidString) < (right, rhs.id.uuidString)
    }
}

/// What the person reviews after a sync: the records that couldn't merge.
/// What reconciling fixed shows in the sync banner instead.
package enum Review {
    package struct Item: Hashable, Sendable, Identifiable {
        package var conflict: SyncConflict
        /// The other device's name.
        package var device: String
        package var id: RecordID { conflict.record }

        package init(conflict: SyncConflict, device: String) {
            self.conflict = conflict
            self.device = device
        }
    }

    /// For each record, the side chosen for each field in conflict. A
    /// conflict about the whole record uses `.deletedAt`.
    package typealias Choices = [RecordID: [Field: SyncSide]]
}
