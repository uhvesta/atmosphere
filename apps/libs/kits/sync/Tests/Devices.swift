import Foundation
import JournalKit
import SyncKit

/// A note's history, as the notes app logs it.
struct Edit: SyncLog {
    var id: UUID
    var summary: String
    var cloudName: String { "edit/\(id.uuidString)" }
}

/// The sample notes app's records on one device.
struct Notebook: SyncSnapshot {
    var items: [RecordID<NoteKind>: Item] = [:]
    var logs: [Edit] = []

    var recordIDs: Set<RecordID<NoteKind>> { Set(items.keys) }
    func value(of record: RecordID<NoteKind>) -> Item? { items[record] }

    static func cloudName(of record: RecordID<NoteKind>) -> String { "\(record.kind.rawValue)/\(record.id)" }

    /// Folders before notes.
    static func sendsBefore(_ lhs: RecordID<NoteKind>, _ rhs: RecordID<NoteKind>) -> Bool {
        (lhs.kind == .note ? 1 : 0, lhs.id.uuidString) < (rhs.kind == .note ? 1 : 0, rhs.id.uuidString)
    }

    mutating func put(_ note: Note) { items[RecordID(.note, note.id)] = .note(note) }

    func note(_ id: UUID) throws -> Note {
        guard case .note(let note)? = items[RecordID(.note, id)] else { throw Unexpected("no note \(id)") }
        return note
    }
}

/// One device: its notebook, kept in memory, synced through a shared cloud.
final class Device: Sendable {
    let name: String
    let syncer: Syncer<Notebook>
    private let notebook: Mutexed<Notebook>

    init(_ name: String, cloud: InMemoryCloud<Notebook>, notebook: Notebook = Notebook()) {
        self.name = name
        let stored = Mutexed(notebook)
        self.notebook = stored
        let store = SyncStore<Notebook>(
            load: { stored.value },
            importSynced: { synced in
                stored.update { notebook in
                    for (id, value) in synced.records { notebook.items[id] = value }
                    notebook.logs += synced.logs
                }
            },
            resolve: { resolved in
                _ = stored.update { notebook in for (id, value) in resolved { notebook.items[id] = value } }
            })
        syncer = Syncer(cloud: cloud.connection(device: name), store: store, storage: .inMemory())
    }

    var records: Notebook { notebook.value }

    func edit(_ change: (inout Notebook) -> Void) {
        _ = notebook.update(change)
    }
}

final class Mutexed<Value: Sendable>: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: Value

    init(_ value: Value) { stored = value }

    var value: Value { lock.withLock { stored } }

    @discardableResult
    func update(_ change: (inout Value) -> Void) -> Value {
        lock.withLock {
            change(&stored)
            return stored
        }
    }
}
