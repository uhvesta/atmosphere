import Foundation

/// Everything in Lifr, as one file: every record, with comments, and the
/// tasks' history. Lifr reads it back with `Workspace.importPlan`.
package struct LifrExport: Codable, Hashable, Sendable {
    /// The newest layout this version reads.
    package static let currentFormat = 1

    package var format: Int
    package var exportedAt: Date
    /// Avenues first, then what goes in them, so a reader meets an
    /// Avenue before its tasks.
    package var records: [RecordValue]
    package var history: [HistoryEvent]

    package init(
        format: Int = Self.currentFormat, exportedAt: Date, records: [RecordValue], history: [HistoryEvent]
    ) {
        self.format = format
        self.exportedAt = exportedAt
        self.records = records
        self.history = history
    }

    /// The file's bytes: sorted keys and dates as ISO 8601, so the same
    /// records always make the same file.
    package func encoded() throws(ImportError) -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        encoder.dateEncodingStrategy = .iso8601
        do {
            return try encoder.encode(self)
        } catch {
            throw .unwritable(String(describing: error))
        }
    }

    /// Reads a file written by `encoded`. A record that can't be read
    /// stops the import, named by its place in the file.
    package static func decoded(from data: Data) throws(ImportError) -> LifrExport {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let envelope: Envelope
        do {
            envelope = try decoder.decode(Envelope.self, from: data)
        } catch let error as DecodingError {
            throw ImportError(error, in: data)
        } catch {
            throw .unreadable(line: nil, String(describing: error))
        }
        guard envelope.format <= currentFormat else { throw .newerFormat(envelope.format) }
        var records: [RecordValue] = []
        for (index, record) in envelope.records.enumerated() {
            switch record.value {
            case .success(let value): records.append(value)
            case .failure(let reason):
                throw .badRecord(number: index + 1, name: record.name, .unreadable(reason.description))
            }
        }
        return LifrExport(
            format: envelope.format, exportedAt: envelope.exportedAt, records: records, history: envelope.history)
    }

    /// The file as read, with each record kept apart so one bad record is
    /// named rather than failing the whole file.
    private struct Envelope: Decodable {
        var format: Int
        var exportedAt: Date
        var records: [LooseRecord]
        var history: [HistoryEvent]
    }

    private struct LooseRecord: Decodable {
        var value: Result<RecordValue, Reason>
        /// The record's title or name, when it has one, to name it by.
        var name: String?

        struct Reason: Error, CustomStringConvertible {
            var description: String
        }

        private struct Named: Decodable {
            var title: String?
            var name: String?
        }

        init(from decoder: any Decoder) throws {
            let container = try decoder.singleValueContainer()
            do {
                value = .success(try container.decode(RecordValue.self))
            } catch {
                value = .failure(Reason(description: ImportError.describe(error)))
            }
            // A record is `{"task": {"_0": {...}}}`: its one value's title
            // or name.
            let named = try? container.decode([String: [String: Named]].self)
            let inner = named?.values.first?.values.first
            name = inner?.title ?? inner?.name
        }
    }
}

/// Why a file couldn't be imported. Nothing is imported when any of it
/// fails.
package enum ImportError: Error, Hashable, Sendable, CustomStringConvertible {
    /// It isn't JSON, or not Lifr's, at `line` when known.
    case unreadable(line: Int?, String)
    /// A newer Lifr wrote it.
    case newerFormat(Int)
    /// The `number`th record is broken or refers to something missing.
    case badRecord(number: Int, name: String?, Problem)
    /// A line of a CSV file can't be read.
    case badLine(Int, Problem)
    /// The export couldn't be written.
    case unwritable(String)

    package enum Problem: Hashable, Sendable {
        case unreadable(String)
        /// Its Avenue isn't here or in the file.
        case missingAvenue
        case missingProject
        /// The task it belongs to isn't here or in the file.
        case missingTask
        /// The CSV row has no title.
        case noTitle
        /// The due date isn't a date.
        case badDate(String)
        /// No Avenue has this name or key.
        case unknownAvenue(String)
        /// No status has this name.
        case unknownStatus(String)
        /// It has fewer columns than the mapping uses.
        case missingColumn(Int)
        /// A quoted field isn't closed.
        case unclosedQuote
    }

    package var description: String {
        switch self {
        case .unreadable(let line?, let reason):
            String(
                localized: """
                    Line \(line) of the file can't be read (\(reason)). Check it's a file Lifr exported, \
                    then try again. Nothing was imported.
                    """)
        case .unreadable(nil, let reason):
            String(
                localized: """
                    The file can't be read (\(reason)). Check it's a file Lifr exported, then try again. \
                    Nothing was imported.
                    """)
        case .newerFormat:
            String(localized: "A newer Lifr made this file. Update Lifr, then import it again. Nothing was imported.")
        case .badRecord(let number, let name?, let problem):
            String(
                localized: """
                    Record \(number) (“\(name)”) \(problem.description). Fix or remove that record in the file, \
                    then import it again. Nothing was imported.
                    """)
        case .badRecord(let number, nil, let problem):
            String(
                localized: """
                    Record \(number) \(problem.description). Fix or remove that record in the file, then import \
                    it again. Nothing was imported.
                    """)
        case .badLine(let line, let problem):
            String(
                localized: """
                    Line \(line) \(problem.description). Fix that line, or pick other columns, then import again. \
                    Nothing was imported.
                    """)
        case .unwritable(let reason):
            String(localized: "The export couldn't be written (\(reason)). Free some space, then try again.")
        }
    }

    /// Where a decoding error happened, by line when the JSON itself is
    /// broken.
    init(_ error: DecodingError, in data: Data) {
        if case .dataCorrupted(let context) = error,
            let index = (context.underlyingError as? NSError)?.userInfo["NSJSONSerializationErrorIndex"] as? Int
        {
            let line = data.prefix(index).count { $0 == UInt8(ascii: "\n") } + 1
            self = .unreadable(line: line, Self.describe(error))
        } else {
            self = .unreadable(line: nil, Self.describe(error))
        }
    }

    static func describe(_ error: any Error) -> String {
        guard let error = error as? DecodingError else { return String(describing: error) }
        switch error {
        case .keyNotFound(let key, _): return String(localized: "“\(key.stringValue)” is missing")
        case .typeMismatch(_, let context), .valueNotFound(_, let context), .dataCorrupted(let context):
            let path = context.codingPath.map(\.stringValue).filter { !$0.hasPrefix("_") }.joined(separator: ".")
            return path.isEmpty
                ? context.debugDescription : String(localized: "“\(path)”: \(context.debugDescription)")
        @unknown default: return String(describing: error)
        }
    }
}

extension ImportError.Problem: CustomStringConvertible {
    package var description: String {
        switch self {
        case .unreadable(let reason): String(localized: "can't be read: \(reason)")
        case .missingAvenue: String(localized: "is in an Avenue that's neither here nor in the file")
        case .missingProject: String(localized: "is in a Project that's neither here nor in the file")
        case .missingTask: String(localized: "belongs to a task that's neither here nor in the file")
        case .noTitle: String(localized: "has no title")
        case .badDate(let text): String(localized: "has a due date, “\(text)”, that isn't YYYY-MM-DD")
        case .unknownAvenue(let name): String(localized: "names an Avenue, “\(name)”, that isn't here")
        case .unknownStatus(let name): String(localized: "names a status, “\(name)”, Lifr doesn't have")
        case .missingColumn(let column): String(localized: "has no column \(column)")
        case .unclosedQuote: String(localized: "has a quote that isn't closed")
        }
    }
}

/// What importing a file would add: only records that aren't here, so
/// importing it again adds nothing.
package struct ImportPlan: Hashable, Sendable {
    package var records: [RecordValue]
    package var history: [HistoryEvent]
    /// Records in the file that are here already, left as they are.
    package var alreadyHere: Int

    /// How many of each kind it adds, for the preview.
    package func count(of kind: RecordKind) -> Int {
        records.count { $0.id.kind == kind }
    }

    package var isEmpty: Bool { records.isEmpty }
}

extension Workspace {
    /// Every record but the settings, which belong to each device's
    /// choices, and the tasks' history.
    package func exported(at now: Date) -> LifrExport {
        func sorted<Record: Identifiable>(_ records: [UUID: Record]) -> [Record] where Record.ID == UUID {
            records.values.sorted { $0.id.uuidString < $1.id.uuidString }
        }
        var records: [RecordValue] = sorted(avenues).map(RecordValue.avenue)
        records += sorted(labels).map(RecordValue.label)
        records += sorted(projects).map(RecordValue.project)
        // Parents before their Sub tasks.
        let tasks = sorted(self.tasks)
        records += tasks.filter { $0.parentID == nil }.map(RecordValue.task)
        records += tasks.filter { $0.parentID != nil }.map(RecordValue.task)
        records += sorted(comments).map(RecordValue.comment)
        records += sorted(reminders).map(RecordValue.reminder)
        records += sorted(savedFilters).map(RecordValue.savedFilter)
        let history = history.sorted { $0.id.uuidString < $1.id.uuidString }
        return LifrExport(exportedAt: now, records: records, history: history)
    }

    /// What importing `export` adds, or the first record that would break
    /// the records, by its place in the file.
    package func importPlan(_ export: LifrExport) throws(ImportError) -> ImportPlan {
        var avenueIDs = Set(avenues.keys)
        var projectIDs = Set(projects.keys)
        var taskIDs = Set(tasks.keys)
        for record in export.records {
            switch record {
            case .avenue(let avenue): avenueIDs.insert(avenue.id)
            case .project(let project): projectIDs.insert(project.id)
            case .task(let task): taskIDs.insert(task.id)
            default: break
            }
        }
        var plan = ImportPlan(records: [], history: [], alreadyHere: 0)
        for (index, record) in export.records.enumerated() {
            let problem: ImportError.Problem?
            let name: String?
            switch record {
            case .avenue(let avenue):
                (problem, name) = (nil, avenue.name)
            case .project(let project):
                (problem, name) = (avenueIDs.contains(project.avenueID) ? nil : .missingAvenue, project.name)
            case .task(let task):
                name = task.title
                if !avenueIDs.contains(task.avenueID) {
                    problem = .missingAvenue
                } else if let project = task.projectID, !projectIDs.contains(project) {
                    problem = .missingProject
                } else if let parent = task.parentID, !taskIDs.contains(parent) {
                    problem = .missingTask
                } else {
                    problem = nil
                }
            case .comment(let comment):
                (problem, name) = (taskIDs.contains(comment.taskID) ? nil : .missingTask, nil)
            case .reminder(let reminder):
                (problem, name) = (taskIDs.contains(reminder.taskID) ? nil : .missingTask, nil)
            case .label(let label):
                (problem, name) = (nil, label.name)
            case .savedFilter(let filter):
                (problem, name) = (nil, filter.name)
            case .settings:
                (problem, name) = (nil, nil)
            }
            if let problem { throw .badRecord(number: index + 1, name: name, problem) }
            if case .settings = record { continue }
            if value(of: record.id) != nil {
                plan.alreadyHere += 1
            } else {
                plan.records.append(record)
            }
        }
        let known = Set(history.map(\.id))
        plan.history = export.history.filter { !known.contains($0.id) && taskIDs.contains($0.taskID) }
        return plan
    }

    /// Adds what `plan` holds that still isn't here, then fixes what it
    /// clashes with, such as a task code or Avenue key in use, as a sync
    /// would.
    package mutating func apply(_ plan: ImportPlan, in context: Context) -> Change {
        var change = Change()
        for record in plan.records where value(of: record.id) == nil {
            set(record.id, to: record)
            switch record {
            case .avenue(let avenue): change.added.append(.avenue(avenue.id))
            case .project(let project): change.added.append(.project(project.id))
            case .label(let label): change.added.append(.label(label.id))
            case .task(let task):
                change.added.append(.task(task.id))
                change.previous[task.id] = .some(nil)
            case .comment(let comment): change.added.append(.comment(comment.id))
            case .reminder(let reminder): change.added.append(.reminder(reminder.id))
            case .settings, .savedFilter: break
            }
        }
        let known = Set(history.map(\.id))
        history += plan.history.filter { !known.contains($0.id) }
        _ = reconcile(in: context)
        return change
    }
}
