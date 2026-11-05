import Foundation

/// This device's version of a record, or the one another device sent.
package enum SyncSide: String, Hashable, Codable, Sendable, CaseIterable {
    case mine, theirs
}

/// A record two devices changed in ways that can't both stand, for the
/// person to review.
package struct SyncConflict: Hashable, Codable, Sendable {
    package enum Kind: Hashable, Codable, Sendable {
        /// Both changed these fields to different values. Placement (a
        /// task's Avenue, Project and parent) is listed as `.avenue`.
        case fields([Field])
        /// One side deleted the record (a task or comment's Deleted, or the
        /// record removed) and the other edited it.
        case removed(by: SyncSide)
    }

    package var record: RecordID
    package var kind: Kind
    package var mine: RecordValue?
    package var theirs: RecordValue?
    /// This device's version with every field that did merge.
    package var merged: RecordValue?

    /// The record with each conflicting field taken from the side chosen
    /// for it, and `.mine` for a field with no choice. A conflict about the
    /// whole record takes the side chosen for `.deletedAt`.
    package func resolved(_ choices: [Field: SyncSide]) -> RecordValue? {
        switch kind {
        case .removed:
            return resolved(keeping: choices[.deletedAt] ?? .mine)
        case .fields(let fields):
            guard var value = merged, let theirs else { return merged }
            for field in fields where choices[field] == .theirs {
                for member in RecordValue.group(of: field, in: record.kind) {
                    value = value.copying(member, from: theirs)
                }
            }
            return value
        }
    }

    /// For a field in conflict, each field it stands for (a task's whole
    /// placement for `.avenue`) with both sides' values, nil when unset.
    package func sides(of field: Field) -> [(field: Field, mine: FieldValue?, theirs: FieldValue?)] {
        let mine = mine?.fieldValues ?? [:]
        let theirs = theirs?.fieldValues ?? [:]
        return RecordValue.group(of: field, in: record.kind)
            .map { ($0, mine[$0], theirs[$0]) }
            .filter { $0.mine != nil || $0.theirs != nil }
    }

    /// The record's name for people, from whichever side has one.
    package var title: String? {
        for value in [mine, theirs, merged].compactMap({ $0 }) {
            switch value {
            case .avenue(let avenue): return avenue.name
            case .project(let project): return project.name
            case .task(let task): return task.title
            case .label(let label): return label.name
            case .comment(let comment): return comment.body
            case .reminder, .settings: continue
            }
        }
        return nil
    }

    /// One side's version of the whole record.
    package func resolved(keeping side: SyncSide) -> RecordValue? {
        switch (kind, side) {
        case (.removed, .mine): mine
        case (.removed, .theirs): theirs
        case (.fields(let fields), _): resolved(Dictionary(uniqueKeysWithValues: fields.map { ($0, side) }))
        }
    }
}

/// What merging two versions of a record gave.
package enum MergeResult: Hashable, Sendable {
    /// The record as both devices should have it, nil when it's removed.
    case merged(RecordValue?)
    case conflict(SyncConflict)
}

extension RecordValue {
    /// Merges this device's and another device's version of a record, field
    /// by field, from `base`: the version both last agreed on, or nil when
    /// they never did. A field changed on one side takes that change; a
    /// field both changed to different values is a conflict. Placement
    /// counts as one field, and an edit against a delete is a conflict about
    /// the whole record.
    package static func merge(base: RecordValue?, mine: RecordValue?, theirs: RecordValue?) -> MergeResult {
        if mine == theirs || theirs == base { return .merged(mine) }
        if mine == base { return .merged(theirs) }
        let record = mine?.id ?? theirs?.id ?? base?.id ?? .settings
        guard let mine, let theirs else {
            // Removed on one side and changed on the other.
            let conflict = SyncConflict(
                record: record, kind: .removed(by: mine == nil ? .mine : .theirs), mine: mine, theirs: theirs,
                merged: mine)
            return .conflict(conflict)
        }
        let old = base?.fieldValues ?? [:]
        let mineTrashed = mine.isTrashed && base?.isTrashed != true
        let theirsTrashed = theirs.isTrashed && base?.isTrashed != true
        if mineTrashed != theirsTrashed {
            // Trashed on one side: a conflict if the other changed anything.
            let editor = mineTrashed ? theirs.fieldValues : mine.fieldValues
            if editor != old {
                return .conflict(
                    SyncConflict(
                        record: record, kind: .removed(by: mineTrashed ? .mine : .theirs), mine: mine,
                        theirs: theirs, merged: mine))
            }
        }

        let ours = mine.fieldValues
        let other = theirs.fieldValues
        var merged = mine
        var conflicts: [Field] = []
        var seen = Set<Field>()
        for field in fields(of: record.kind) where !seen.contains(field) {
            let group = group(of: field, in: record.kind)
            seen.formUnion(group)
            let was = group.map { old[$0] }
            let now = group.map { ours[$0] }
            let sent = group.map { other[$0] }
            // Both trashed: either date will do.
            if field == .deletedAt, mineTrashed, theirsTrashed { continue }
            if now == sent || sent == was { continue }
            if now == was {
                for member in group { merged = merged.copying(member, from: theirs) }
            } else {
                conflicts.append(field)
            }
        }
        merged = merged.withLatestActivity(of: theirs)
        guard conflicts.isEmpty else {
            return .conflict(
                SyncConflict(record: record, kind: .fields(conflicts), mine: mine, theirs: theirs, merged: merged))
        }
        return .merged(merged)
    }

    /// The fields that merge as one with `field`.
    static func group(of field: Field, in kind: RecordKind) -> [Field] {
        let placement: [Field] = [.avenue, .project, .parent]
        return kind == .task && placement.contains(field) ? placement : [field]
    }

    /// Whether the record is in the trash.
    var isTrashed: Bool { fieldValues[.deletedAt] != nil }

    /// This record with the later activity times of the two versions.
    func withLatestActivity(of other: RecordValue) -> RecordValue {
        guard case .task(var task) = self, case .task(let theirs) = other else { return self }
        task.updatedAt = max(task.updatedAt, theirs.updatedAt)
        task.lastActivityAt = max(task.lastActivityAt, theirs.lastActivityAt)
        return .task(task)
    }
}

extension Workspace {
    /// Writes what sync brought in as it is, without a journal entry: the
    /// other device's own entries arrive with its changes. A nil value
    /// removes the record. Journal entries and history events are added
    /// unless they're here already. Call `reconcile` once the import is
    /// done.
    package mutating func importSynced(
        records: [RecordID: RecordValue?] = [:], journal synced: [JournalEntry] = [],
        history syncedHistory: [HistoryEvent] = []
    ) {
        for (record, value) in records.sorted(by: { $0.key.kind.order < $1.key.kind.order }) {
            set(record, to: value)
        }
        if !synced.isEmpty {
            let known = Set(journal.map(\.id))
            journal += synced.filter { !known.contains($0.id) }
            journal.sort { ($0.at, $0.id.uuidString) < ($1.at, $1.id.uuidString) }
        }
        if !syncedHistory.isEmpty {
            let known = Set(history.map(\.id))
            history += syncedHistory.filter { !known.contains($0.id) }
            history.sort { ($0.createdAt, $0.id.uuidString) < ($1.createdAt, $1.id.uuidString) }
        }
    }

    /// Writes the person's choices for conflicting records. Run it with
    /// `perform`, so the choices are one entry that can be undone.
    package mutating func resolveSynced(_ records: [RecordID: RecordValue?]) -> Change {
        for (record, value) in records.sorted(by: { $0.key.kind.order < $1.key.kind.order }) {
            set(record, to: value)
        }
        return Change()
    }
}

extension RecordKind {
    /// Records others refer to come first.
    fileprivate var order: Int { Self.allCases.firstIndex(of: self) ?? 0 }
}
