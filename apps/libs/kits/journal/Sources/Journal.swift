import Foundation

/// One operation's changes to the records, with who made them. Written once,
/// never changed: reverting an entry appends another, so it can be reverted
/// too.
public struct JournalEntry<Record: Journaled, Author: Hashable & Codable & Sendable>: Identifiable, Hashable, Codable,
    Sendable
{
    public var id: UUID
    public var actor: Author
    public var at: Date
    /// The conversation the change was made in, such as one with an
    /// assistant.
    public var conversation: UUID?
    /// The entries this one reverts, newest first.
    public var reverts: [UUID]
    public var steps: [JournalStep<Record>]

    public init(
        id: UUID, actor: Author, at: Date, conversation: UUID? = nil, reverts: [UUID] = [],
        steps: [JournalStep<Record>]
    ) {
        self.id = id
        self.actor = actor
        self.at = at
        self.conversation = conversation
        self.reverts = reverts
        self.steps = steps
    }

    /// Newest first, with ties broken by ID so every device orders the
    /// same.
    public static func newestFirst(_ lhs: Self, _ rhs: Self) -> Bool {
        (lhs.at, lhs.id.uuidString) > (rhs.at, rhs.id.uuidString)
    }
}

/// One record as it was before and after an operation. `before` is nil for
/// a record the operation added, `after` for one it removed.
public struct JournalStep<Record: Journaled>: Hashable, Codable, Sendable {
    public private(set) var before: Record?
    public private(set) var after: Record?

    public init(before: Record?, after: Record?) {
        precondition(before != nil || after != nil, "A step needs the record before or after")
        self.before = before
        self.after = after
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        before = try container.decodeIfPresent(Record.self, forKey: .before)
        after = try container.decodeIfPresent(Record.self, forKey: .after)
        guard before != nil || after != nil else {
            throw DecodingError.dataCorruptedError(
                forKey: .after, in: container, debugDescription: "A step needs the record before or after")
        }
    }

    public var record: RecordID<Record.Kind> {
        if let after { return after.id }
        if let before { return before.id }
        preconditionFailure("A step needs the record before or after")
    }

    /// The fields that differ, in the order people see them.
    public var fields: [FieldChange<Record>] {
        let old = before?.fieldValues ?? [:]
        let new = after?.fieldValues ?? [:]
        return Record.fields(of: record.kind).compactMap { field in
            old[field] == new[field] ? nil : FieldChange(field: field, before: old[field], after: new[field])
        }
    }

    /// A step for each record that differs between `old` and `new`, by ID.
    public static func steps<Element: Equatable>(
        from old: [UUID: Element], to new: [UUID: Element], as record: (Element) -> Record
    ) -> [Self] {
        Set(old.keys).union(new.keys).sorted { $0.uuidString < $1.uuidString }.compactMap { id in
            guard old[id] != new[id] else { return nil }
            return Self(before: old[id].map(record), after: new[id].map(record))
        }
    }
}

/// A field of a record before and after. A value is nil when the field
/// was empty, or the record wasn't there.
public struct FieldChange<Record: Journaled>: Hashable, Sendable {
    public var field: Record.Field
    public var before: Record.Value?
    public var after: Record.Value?

    public init(field: Record.Field, before: Record.Value?, after: Record.Value?) {
        self.field = field
        self.before = before
        self.after = after
    }
}

/// Saved in an archive, when the record's values can be.
extension FieldChange: Codable where Record.Value: Codable {}

extension Sequence {
    /// The entries undone: reverted by an entry that isn't undone itself.
    public func undone<Record, Author>() -> Set<UUID> where Element == JournalEntry<Record, Author> {
        var undone: Set<UUID> = []
        for entry in sorted(by: JournalEntry.newestFirst) where !undone.contains(entry.id) {
            undone.formUnion(entry.reverts)
        }
        return undone
    }

    /// The entries made since `date`, newest first, for "revert everything
    /// since".
    public func made<Record, Author>(since date: Date) -> [Element] where Element == JournalEntry<Record, Author> {
        filter { $0.at >= date }.sorted(by: JournalEntry.newestFirst)
    }

    /// The entries made in `conversation`, newest first.
    public func made<Record, Author>(inConversation conversation: UUID) -> [Element]
    where Element == JournalEntry<Record, Author> {
        filter { $0.conversation == conversation }.sorted(by: JournalEntry.newestFirst)
    }
}

extension Array {
    /// Adds the entries another device sent that aren't here already, and
    /// keeps the journal oldest first.
    public mutating func add<Record, Author>(synced: [Element]) where Element == JournalEntry<Record, Author> {
        guard !synced.isEmpty else { return }
        let known = Set(map(\.id))
        self += synced.filter { !known.contains($0.id) }
        sort { JournalEntry.newestFirst($1, $0) }
    }
}
