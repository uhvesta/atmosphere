import Foundation
import JournalKit

// A small notes app to test the journal on: notes filed in folders, with a
// trash. A note's folder and position merge as one.

struct Note: Hashable, Codable, Sendable {
    var id: UUID
    var title: String
    var body: String = ""
    var folder: UUID?
    var position: Int = 0
    var deletedAt: Date?
    /// Bookkeeping: not a field, so it never conflicts.
    var editedAt: Date = Date(timeIntervalSince1970: 0)
}

struct Folder: Hashable, Codable, Sendable {
    var id: UUID
    var name: String
}

enum NoteKind: String, Hashable, Codable, Sendable {
    case note, folder
}

enum NoteField: String, Hashable, Codable, Sendable {
    case title, body, folder, position, deletedAt, name
}

enum NoteValue: Hashable, Codable, Sendable {
    case text(String)
    case number(Int)
    case folder(UUID)
    case moment(Date)
}

enum Item: Journaled {
    case note(Note)
    case folder(Folder)

    var id: RecordID<NoteKind> {
        switch self {
        case .note(let note): RecordID(.note, note.id)
        case .folder(let folder): RecordID(.folder, folder.id)
        }
    }

    static func fields(of kind: NoteKind) -> [NoteField] {
        switch kind {
        case .note: [.title, .body, .folder, .position, .deletedAt]
        case .folder: [.name]
        }
    }

    var fieldValues: [NoteField: NoteValue] {
        switch self {
        case .note(let note):
            var values: [NoteField: NoteValue] = [.title: .text(note.title), .position: .number(note.position)]
            if !note.body.isEmpty { values[.body] = .text(note.body) }
            if let folder = note.folder { values[.folder] = .folder(folder) }
            if let deletedAt = note.deletedAt { values[.deletedAt] = .moment(deletedAt) }
            return values
        case .folder(let folder):
            return [.name: .text(folder.name)]
        }
    }

    func copying(_ field: NoteField, from other: Item) -> Item {
        switch (self, other) {
        case (.note(var note), .note(let from)):
            switch field {
            case .title: note.title = from.title
            case .body: note.body = from.body
            case .folder: note.folder = from.folder
            case .position: note.position = from.position
            case .deletedAt: note.deletedAt = from.deletedAt
            case .name: break
            }
            return .note(note)
        case (.folder(var folder), .folder(let from)):
            if field == .name { folder.name = from.name }
            return .folder(folder)
        default:
            return self
        }
    }

    static func group(of field: NoteField, in kind: NoteKind) -> [NoteField] {
        kind == .note && [.folder, .position].contains(field) ? [.folder, .position] : [field]
    }

    static var trashField: NoteField? { .deletedAt }

    func keepingBookkeeping(of other: Item) -> Item {
        guard case .note(var note) = self, case .note(let theirs) = other else { return self }
        note.editedAt = max(note.editedAt, theirs.editedAt)
        return .note(note)
    }
}

typealias Entry = JournalEntry<Item, String>
typealias Step = JournalStep<Item>

let shopping = Note(id: UUID(uuid: (1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1)), title: "Shopping")
let ideas = Note(id: UUID(uuid: (1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2)), title: "Ideas")
let home = Folder(id: UUID(uuid: (2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1)), name: "Home")
let work = Folder(id: UUID(uuid: (2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2)), name: "Work")

func at(_ seconds: TimeInterval) -> Date { Date(timeIntervalSince1970: seconds) }

func entry(_ number: UInt8, at seconds: TimeInterval, reverts: [UUID] = [], conversation: UUID? = nil) -> Entry {
    Entry(
        id: UUID(uuid: (3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, number)), actor: "Sam", at: at(seconds),
        conversation: conversation, reverts: reverts, steps: [Step(before: nil, after: .note(shopping))])
}

struct Unexpected: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}
