import Foundation
import JournalKit

/// This device's version of a record, or the one another device sent.
package typealias SyncSide = JournalKit.SyncSide
/// A record two devices changed in ways that can't both stand, for the
/// person to review. A conflict about the whole record is chosen with
/// `.deletedAt`.
package typealias SyncConflict = JournalKit.SyncConflict<RecordValue>
/// What merging two versions of a record gave.
package typealias MergeResult = JournalKit.MergeResult<RecordValue>

extension SyncConflict where Record == RecordValue {
    /// The record's name for people, from whichever side has one.
    package var title: String? {
        for value in [mine, theirs, merged].compactMap({ $0 }) {
            switch value {
            case .avenue(let avenue): return avenue.name
            case .project(let project): return project.name
            case .task(let task): return task.title
            case .label(let label): return label.name
            case .comment(let comment): return comment.body
            case .savedFilter(let saved): return saved.name
            case .reminder, .settings: continue
            }
        }
        return nil
    }
}

extension RecordValue {
    /// A task's placement, its Avenue, Project and parent, merges as one
    /// field, listed as `.avenue`.
    package static func group(of field: Field, in kind: RecordKind) -> [Field] {
        let placement: [Field] = [.avenue, .project, .parent]
        return kind == .task && placement.contains(field) ? placement : [field]
    }

    /// Tasks and comments go to the trash; every other record's conflicts
    /// about the whole record are chosen with it too.
    package static var trashField: Field? { .deletedAt }

    /// A merged task keeps the later activity times of the two versions.
    package func keepingBookkeeping(of other: RecordValue) -> RecordValue {
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
        history syncedHistory: [HistoryEvent] = [], seen: [RecordID: RecordValue?] = [:]
    ) {
        for (record, value) in records.sorted(by: { $0.key.kind.order < $1.key.kind.order }) {
            // Edited here after sync read it: the edit is merged in, and it
            // keeps its values where both changed the same field, so the
            // next sync sends it.
            if let read = seen[record], case let now = self.value(of: record), now != read {
                switch RecordValue.merge(base: read, mine: now, theirs: value) {
                case .merged(let merged): set(record, to: merged)
                case .conflict(let conflict): set(record, to: conflict.merged)
                }
            } else {
                set(record, to: value)
            }
        }
        journal.add(synced: synced)
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
