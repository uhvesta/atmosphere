import Foundation

/// What reverting one step does to its record as it now is.
public enum StepRevert<Record: Journaled>: Hashable, Sendable {
    /// Nothing: the step added the record, and it's gone already.
    case nothing
    /// The step added the record: take it out. Whether it's trashed,
    /// deleted, or kept because others use it is the app's call.
    case remove(Record)
    /// The step removed the record: put it back.
    case restore(Record)
    /// The record changed since and is gone now, so the step can't be put
    /// back.
    case gone
    /// The record with each field the step changed put back, or nil when
    /// none of them is. `skipped` are the fields changed again since, left
    /// as they are so reverting doesn't overwrite that.
    case update(Record?, skipped: [Record.Field])
}

/// A part of an entry that reverting left as it is, and why.
public struct SkippedRevert<Record: Journaled>: Hashable, Sendable {
    public enum Reason: Hashable, Sendable {
        /// The field was changed again since, so reverting would overwrite that.
        case changedSince
        /// The record is gone, such as one purged from the trash.
        case gone
        /// The record would be removed, but other records still use it.
        case inUse
    }

    public var record: RecordID<Record.Kind>
    /// The field, when only one was skipped.
    public var field: Record.Field?
    public var reason: Reason

    public init(record: RecordID<Record.Kind>, field: Record.Field?, reason: Reason) {
        self.record = record
        self.field = field
        self.reason = reason
    }
}

extension JournalStep {
    /// What reverting this step does to `current`, the record as it now is,
    /// field by field.
    public func revert(current: Record?) -> StepRevert<Record> {
        switch (before, after, current) {
        case (nil, _, nil): .nothing
        case (nil, _, let current?): .remove(current)
        case (let before?, nil, nil): .restore(before)
        case (_?, _?, nil): .gone
        case (let before?, let after, let current?): Self.put(current: current, from: after, back: before)
        }
    }

    /// `current` with each field that went from `from` to `to` put back to
    /// `to`, unless it has changed since.
    private static func put(current: Record, from: Record?, back to: Record) -> StepRevert<Record> {
        let old = from?.fieldValues ?? [:]
        let new = to.fieldValues
        let now = current.fieldValues
        var merged = current
        var skipped: [Record.Field] = []
        for field in Record.fields(of: to.id.kind) where old[field] != new[field] && now[field] != new[field] {
            guard now[field] == old[field] else {
                skipped.append(field)
                continue
            }
            merged = merged.copying(field, from: to)
        }
        return .update(merged == current ? nil : merged, skipped: skipped)
    }
}
