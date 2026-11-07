import Foundation

/// This device's version of a record, or the one another device sent.
public enum SyncSide: String, Hashable, Codable, Sendable, CaseIterable {
    case mine, theirs
}

/// A record two devices changed in ways that can't both stand, for the
/// person to review.
public struct SyncConflict<Record: Journaled>: Hashable, Codable, Sendable {
    public enum Kind: Hashable, Codable, Sendable {
        /// Both changed these fields to different values. A group of fields
        /// that merge as one is listed by its first field.
        case fields([Record.Field])
        /// One side removed or trashed the record and the other edited it.
        case removed(by: SyncSide)
    }

    public var record: RecordID<Record.Kind>
    public var kind: Kind
    public var mine: Record?
    public var theirs: Record?
    /// This device's version with every field that did merge.
    public var merged: Record?

    public init(record: RecordID<Record.Kind>, kind: Kind, mine: Record?, theirs: Record?, merged: Record?) {
        self.record = record
        self.kind = kind
        self.mine = mine
        self.theirs = theirs
        self.merged = merged
    }

    /// The record with each conflicting field taken from the side chosen
    /// for it, and `.mine` for a field with no choice. A conflict about the
    /// whole record takes the side chosen for the trash field.
    public func resolved(_ choices: [Record.Field: SyncSide]) -> Record? {
        switch kind {
        case .removed:
            return resolved(keeping: Record.trashField.flatMap { choices[$0] } ?? .mine)
        case .fields(let fields):
            guard var value = merged, let theirs else { return merged }
            for field in fields where choices[field] == .theirs {
                for member in Record.group(of: field, in: record.kind) {
                    value = value.copying(member, from: theirs)
                }
            }
            return value
        }
    }

    /// One side's version of the whole record.
    public func resolved(keeping side: SyncSide) -> Record? {
        switch (kind, side) {
        case (.removed, .mine): mine
        case (.removed, .theirs): theirs
        case (.fields(let fields), _): resolved(Dictionary(uniqueKeysWithValues: fields.map { ($0, side) }))
        }
    }

    /// For a field in conflict, each field of its group with both sides'
    /// values, nil when unset.
    public func sides(of field: Record.Field) -> [(field: Record.Field, mine: Record.Value?, theirs: Record.Value?)] {
        let mine = mine?.fieldValues ?? [:]
        let theirs = theirs?.fieldValues ?? [:]
        return Record.group(of: field, in: record.kind)
            .map { ($0, mine[$0], theirs[$0]) }
            .filter { $0.mine != nil || $0.theirs != nil }
    }
}

/// What merging two versions of a record gave.
public enum MergeResult<Record: Journaled>: Hashable, Sendable {
    /// The record as both devices should have it, nil when it's removed.
    case merged(Record?)
    case conflict(SyncConflict<Record>)
}

extension Journaled {
    /// Merges this device's and another device's version of a record, field
    /// by field, from `base`: the version both last agreed on, or nil when
    /// they never did. A field changed on one side takes that change; a
    /// field both changed to different values is a conflict. A group of
    /// fields merges as one, and an edit against a removal or trashing is a
    /// conflict about the whole record.
    public static func merge(base: Self?, mine: Self?, theirs: Self?) -> MergeResult<Self> {
        if mine == theirs || theirs == base { return .merged(mine) }
        if mine == base { return .merged(theirs) }
        guard let mine, let theirs else {
            // Removed on one side and changed on the other.
            guard let record = (mine ?? theirs)?.id else { return .merged(nil) }
            let conflict = SyncConflict(
                record: record, kind: .removed(by: mine == nil ? .mine : .theirs), mine: mine, theirs: theirs,
                merged: mine)
            return .conflict(conflict)
        }
        let record = mine.id
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
            // Both trashed: either time will do.
            if field == trashField, mineTrashed, theirsTrashed { continue }
            let was = group.map { old[$0] }
            let now = group.map { ours[$0] }
            let sent = group.map { other[$0] }
            if now == sent || sent == was { continue }
            if now == was {
                for member in group { merged = merged.copying(member, from: theirs) }
            } else {
                conflicts.append(field)
            }
        }
        merged = merged.keepingBookkeeping(of: theirs)
        guard conflicts.isEmpty else {
            return .conflict(
                SyncConflict(record: record, kind: .fields(conflicts), mine: mine, theirs: theirs, merged: merged))
        }
        return .merged(merged)
    }
}
