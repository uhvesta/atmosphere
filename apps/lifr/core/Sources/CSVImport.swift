import Foundation

/// A CSV file's rows, the first as the column names.
package struct CSVTable: Hashable, Sendable {
    package var columns: [String]
    /// The rows after the first, each with its line in the file.
    package var rows: [(line: Int, fields: [String])]

    package static func == (lhs: CSVTable, rhs: CSVTable) -> Bool {
        lhs.columns == rhs.columns && lhs.rows.map(\.line) == rhs.rows.map(\.line)
            && lhs.rows.map(\.fields) == rhs.rows.map(\.fields)
    }

    package func hash(into hasher: inout Hasher) {
        hasher.combine(columns)
        for row in rows {
            hasher.combine(row.line)
            hasher.combine(row.fields)
        }
    }

    /// Reads comma-separated text, with fields in double quotes holding
    /// commas, line breaks and doubled quotes. Empty lines are skipped.
    package init(parsing text: String) throws(ImportError) {
        var records: [(line: Int, fields: [String])] = []
        var fields: [String] = []
        var field = ""
        var quoted = false
        var line = 1
        var start = 1
        var characters = text.makeIterator()
        var pending = characters.next()
        while let character = pending {
            pending = characters.next()
            if quoted {
                if character == "\"" {
                    if pending == "\"" {
                        field.append("\"")
                        pending = characters.next()
                    } else {
                        quoted = false
                    }
                } else {
                    if character.isNewline { line += 1 }
                    field.append(character)
                }
                continue
            }
            switch character {
            case "\"" where field.isEmpty:
                quoted = true
            case ",":
                fields.append(field)
                field = ""
            case _ where character.isNewline:
                fields.append(field)
                if fields != [""] { records.append((start, fields)) }
                (fields, field) = ([], "")
                line += 1
                start = line
            default:
                field.append(character)
            }
        }
        if quoted { throw .badLine(start, .unclosedQuote) }
        fields.append(field)
        if fields != [""] { records.append((start, fields)) }
        columns = records.first?.fields.map { $0.trimmingCharacters(in: .whitespaces) } ?? []
        rows = Array(records.dropFirst())
    }
}

/// Which of a CSV file's columns hold what. Only the title is needed.
package struct CSVMapping: Hashable, Sendable {
    package var title: Int
    package var details: Int?
    /// YYYY-MM-DD.
    package var due: Int?
    /// An Avenue's name or key; rows without one go to `avenueID`.
    package var avenue: Int?
    /// A status's name, such as To Do or Done.
    package var status: Int?
    /// Where rows go that don't name an Avenue.
    package var avenueID: UUID

    package init(
        title: Int, details: Int? = nil, due: Int? = nil, avenue: Int? = nil, status: Int? = nil,
        avenueID: UUID = Avenue.inboxID
    ) {
        self.title = title
        self.details = details
        self.due = due
        self.avenue = avenue
        self.status = status
        self.avenueID = avenueID
    }

    /// Guesses from the column names, in English: Title or Name, Notes or
    /// Description, Due, Avenue or List, Status.
    package init?(guessingFrom columns: [String]) {
        func column(_ names: Set<String>) -> Int? {
            columns.firstIndex { names.contains($0.lowercased()) }
        }
        guard let title = column(["title", "name", "task", "content"]) ?? (columns.isEmpty ? nil : 0) else {
            return nil
        }
        self.init(
            title: title, details: column(["details", "notes", "description"]),
            due: column(["due", "due date", "date"]), avenue: column(["avenue", "list", "project"]),
            status: column(["status", "state"]))
    }
}

extension Workspace {
    /// The tasks `table`'s rows make under `mapping`, or the first line
    /// that can't be one.
    package func newTasks(from table: CSVTable, mapping: CSVMapping) throws(ImportError) -> [NewTask] {
        let open = avenues.values.filter { $0.archivedAt == nil }
        var tasks: [NewTask] = []
        for (line, fields) in table.rows {
            func field(_ column: Int?) throws(ImportError) -> String? {
                guard let column else { return nil }
                guard column < fields.count else { throw .badLine(line, .missingColumn(column + 1)) }
                let text = fields[column].trimmingCharacters(in: .whitespacesAndNewlines)
                return text.isEmpty ? nil : text
            }
            guard let title = try field(mapping.title) else { throw .badLine(line, .noTitle) }
            var task = NewTask(title: title, avenueID: mapping.avenueID)
            task.details = try field(mapping.details) ?? ""
            if let due = try field(mapping.due) {
                guard let date = try? CalendarDate(due) else { throw .badLine(line, .badDate(due)) }
                task.dueDate = date
            }
            if let name = try field(mapping.avenue) {
                guard
                    let avenue = open.first(where: {
                        $0.name.localizedCaseInsensitiveCompare(name) == .orderedSame
                            || $0.key.rawValue.caseInsensitiveCompare(name) == .orderedSame
                    })
                else { throw .badLine(line, .unknownAvenue(name)) }
                task.avenueID = avenue.id
            }
            if let name = try field(mapping.status) {
                guard let status = Self.status(named: name) else { throw .badLine(line, .unknownStatus(name)) }
                task.status = status
            }
            tasks.append(task)
        }
        return tasks
    }

    /// A status by its English name, however it's spelt: "In Progress",
    /// "in_progress" and "inprogress" are one. "Completed" and "Closed"
    /// are Done, and "Open" is To Do.
    static func status(named name: String) -> TaskStatus? {
        let letters = String(name.lowercased().filter(\.isLetter))
        switch letters {
        case "completed", "complete", "closed", "finished": return .done
        case "open", "notstarted": return .todo
        case "cancelled", "canceled": return .wontDo
        default: return TaskStatus.allCases.first { $0.rawValue.lowercased() == letters }
        }
    }
}
