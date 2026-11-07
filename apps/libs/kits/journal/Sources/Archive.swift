import Foundation

/// A journal entry kept once it's too old to undo: what each step changed,
/// field by field, without the whole records. It can be looked back at but
/// not reverted, and takes far less room than the entry.
public struct ArchivedEntry<Record: Journaled, Author: Hashable & Codable & Sendable>: Identifiable, Hashable,
    Sendable
{
    public var id: UUID
    public var actor: Author
    public var at: Date
    public var conversation: UUID?
    /// The entries this one reverted, newest first.
    public var reverts: [UUID]
    public var steps: [ArchivedStep<Record>]

    public init(
        id: UUID, actor: Author, at: Date, conversation: UUID? = nil, reverts: [UUID] = [],
        steps: [ArchivedStep<Record>]
    ) {
        self.id = id
        self.actor = actor
        self.at = at
        self.conversation = conversation
        self.reverts = reverts
        self.steps = steps
    }
}

/// What one step did to its record, as the fields it changed.
public struct ArchivedStep<Record: Journaled>: Hashable, Sendable {
    public enum Change: String, Hashable, Codable, Sendable {
        /// The step added the record; its fields are every field it had.
        case added
        /// The step removed the record; its fields are every field it had.
        case removed
        case changed
    }

    public var record: RecordID<Record.Kind>
    public var change: Change
    public var fields: [FieldChange<Record>]

    public init(record: RecordID<Record.Kind>, change: Change, fields: [FieldChange<Record>]) {
        self.record = record
        self.change = change
        self.fields = fields
    }
}

extension ArchivedEntry: Codable where Record.Value: Codable {}
extension ArchivedStep: Codable where Record.Value: Codable {}

extension JournalEntry {
    /// The entry with only the fields each step changed.
    public var archived: ArchivedEntry<Record, Author> {
        ArchivedEntry(
            id: id, actor: actor, at: at, conversation: conversation, reverts: reverts,
            steps: steps.map { step in
                let change: ArchivedStep<Record>.Change =
                    step.before == nil ? .added : step.after == nil ? .removed : .changed
                return ArchivedStep(record: step.record, change: change, fields: step.fields)
            })
    }
}

/// A calendar month in UTC, which an archive holds the entries of. Every
/// device puts an entry in the same month, wherever it is.
public struct JournalMonth: Hashable, Comparable, Codable, Sendable, CustomStringConvertible {
    public var year: Int
    public var month: Int

    public init(year: Int, month: Int) {
        self.year = year
        self.month = month
    }

    /// The month `date` is in.
    public init(containing date: Date) {
        let parts = Self.calendar.dateComponents([.year, .month], from: date)
        self.init(year: parts.year ?? 1970, month: parts.month ?? 1)
    }

    /// Reads "2026-07", as `description` writes it; nil for anything else.
    public init?(_ text: String) {
        let parts = text.split(separator: "-")
        guard parts.count == 2, let year = Int(parts[0]), let month = Int(parts[1]), (1...12).contains(month)
        else { return nil }
        self.init(year: year, month: month)
    }

    /// "2026-07".
    public var description: String {
        String(year).leftPadded(to: 4) + "-" + String(month).leftPadded(to: 2)
    }

    /// The month's first moment.
    public var start: Date {
        Self.calendar.date(from: DateComponents(year: year, month: month, day: 1)) ?? .distantPast
    }

    public static func < (lhs: Self, rhs: Self) -> Bool { (lhs.year, lhs.month) < (rhs.year, rhs.month) }

    private static let calendar: Calendar = {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC") ?? calendar.timeZone
        return calendar
    }()
}

extension String {
    fileprivate func leftPadded(to width: Int) -> String {
        String(repeating: "0", count: Swift.max(0, width - count)) + self
    }
}

extension Array {
    /// The entries made before `cutoff`, archived by month, and the rest,
    /// which stay as they are.
    public func archiving<Record, Author>(
        before cutoff: Date
    ) -> (kept: [Element], archived: [JournalMonth: [ArchivedEntry<Record, Author>]])
    where Element == JournalEntry<Record, Author> {
        var kept: [Element] = []
        var archived: [JournalMonth: [ArchivedEntry<Record, Author>]] = [:]
        for entry in self {
            if entry.at < cutoff {
                archived[JournalMonth(containing: entry.at), default: []].append(entry.archived)
            } else {
                kept.append(entry)
            }
        }
        return (kept, archived)
    }

    /// The archived entries with `other`'s added, each once, oldest first:
    /// two devices can archive the same entry.
    public func merging<Record, Author>(_ other: [Element]) -> [Element]
    where Element == ArchivedEntry<Record, Author> {
        Dictionary((self + other).map { ($0.id, $0) }) { first, _ in first }.values
            .sorted { ($0.at, $0.id.uuidString) < ($1.at, $1.id.uuidString) }
    }
}
