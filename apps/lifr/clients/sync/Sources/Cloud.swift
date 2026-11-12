import Foundation
import LifrCore
import LifrStoreClient
import SyncKit

// Lifr syncs through SyncKit; these are its names for SyncKit's types.

package typealias CloudItem = SyncKit.CloudItem<SyncedRecords>
package typealias CloudChange = SyncKit.CloudChange<SyncedRecords>
package typealias Cloud = SyncKit.Cloud<SyncedRecords>
package typealias SyncError = SyncKit.SyncError
package typealias SyncLedger = SyncKit.SyncLedger<SyncedRecords>
package typealias LedgerStorage = SyncKit.LedgerStorage<SyncedRecords>
package typealias InMemoryCloud = SyncKit.InMemoryCloud<SyncedRecords>
package typealias SyncState = SyncKit.SyncState<RecordValue>

/// What the person reviews after a sync: the records that couldn't merge.
/// What reconciling fixed shows in the sync banner instead.
package enum Review {
    package typealias Item = ReviewItem<RecordValue>
    /// For each record, the side chosen for each field in conflict. A
    /// conflict about the whole record uses `.deletedAt`.
    package typealias Choices = ReviewChoices<RecordValue>
}

/// A journal entry or history event: written once, never changed, so each
/// device only adds the ones it doesn't have. Saved as it was before it was
/// a log, such as `{"entry": {"_0": ...}}`.
package enum SyncedLog: SyncLog {
    case entry(JournalEntry)
    case event(HistoryEvent)

    package var id: UUID {
        switch self {
        case .entry(let entry): entry.id
        case .event(let event): event.id
        }
    }

    package var cloudName: String {
        switch self {
        case .entry(let entry): "journal/\(entry.id.uuidString)"
        case .event(let event): "history/\(event.id.uuidString)"
        }
    }
}

/// The workspace as sync sees it.
package struct SyncedRecords: SyncSnapshot {
    package var workspace: Workspace

    package init(_ workspace: Workspace) {
        self.workspace = workspace
    }

    package var recordIDs: Set<RecordID> {
        var records: Set<RecordID> = [.settings]
        records.formUnion(workspace.avenues.keys.map { RecordID(.avenue, $0) })
        records.formUnion(workspace.projects.keys.map { RecordID(.project, $0) })
        records.formUnion(workspace.tasks.keys.map { RecordID(.task, $0) })
        records.formUnion(workspace.labels.keys.map { RecordID(.label, $0) })
        records.formUnion(workspace.comments.keys.map { RecordID(.comment, $0) })
        records.formUnion(workspace.reminders.keys.map { RecordID(.reminder, $0) })
        records.formUnion(workspace.savedFilters.keys.map { RecordID(.savedFilter, $0) })
        return records
    }

    package func value(of record: RecordID) -> RecordValue? { workspace.value(of: record) }

    package var unreadable: Set<RecordID> { workspace.unreadable }

    package var logIDs: Set<UUID> {
        Set(workspace.journal.map(\.id)).union(workspace.history.map(\.id))
    }

    package var logs: [SyncedLog] {
        workspace.journal.map(SyncedLog.entry) + workspace.history.map(SyncedLog.event)
    }

    package static func cloudName(of record: RecordID) -> String {
        record.kind == .settings ? "settings" : "\(record.kind.rawValue)/\(record.id.uuidString)"
    }

    /// Records others refer to first, then by ID.
    package static func sendsBefore(_ lhs: RecordID, _ rhs: RecordID) -> Bool {
        let kinds = RecordKind.allCases
        let left = kinds.firstIndex(of: lhs.kind) ?? 0
        let right = kinds.firstIndex(of: rhs.kind) ?? 0
        return (left, lhs.id.uuidString) < (right, rhs.id.uuidString)
    }
}

extension SyncedChanges {
    /// What SyncKit merged in, as the store writes it.
    package init(_ synced: Synced<SyncedRecords>) {
        var journal: [JournalEntry] = []
        var history: [HistoryEvent] = []
        for log in synced.logs {
            switch log {
            case .entry(let entry): journal.append(entry)
            case .event(let event): history.append(event)
            }
        }
        self.init(records: synced.records, journal: journal, history: history)
    }
}
