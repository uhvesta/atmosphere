import Foundation
import LifrCore

/// Plans as TOML, the form people and agents read and edit. Parents come
/// before their children: a `[[project.task]]` belongs to the `[[project]]`
/// above it, and a `[[task.subtask]]` to the `[[task]]` above it.
///
/// Only the TOML a plan needs is read: `[[...]]` headers, strings (basic,
/// literal and multi-line), arrays of strings, bare dates and comments. That
/// keeps this module free of third-party code, so the CLI stays hermetic.
package enum PlanFile {
    package static let firstLine = "# Lifr plan v1"

    /// The tables and fields, for agents writing a plan.
    package static let guide = """
        \(firstLine)
        [[avenue]]               key, name, color, symbol
        [[label]]                name, color
        [[project]]              name, avenue (key), details, due
          [[project.task]]       task fields
            [[project.task.subtask]]
        [[task]]                 task fields, plus in = "KEY" or "KEY/Project name", or parent = "TASK-CODE"
          [[task.subtask]]       task fields

        Task fields: title (required), details, status (todo, in_progress, procrastinated, done, wont_do),
        priority (none, low, medium, high, urgent), start, due, due_time ("HH:MM"), repeats, labels, remind,
        comments.
        Dates: YYYY-MM-DD, today, tomorrow, a weekday, next friday, +3d, +2w, +1m.
        repeats (not on Sub tasks): "every day", "every weekday", "every 2 weeks on Mon, Thu",
        "every month on day 15", "every month on the last Fri", "every year on Mar 5", "3 days after closing".
        remind: "1 day before due", "2 hours before due", or a date with a time, such as "tomorrow 09:00".
        """

    // MARK: - Writing

    /// The plan as TOML, the same each time for the same plan. Fields left
    /// at their defaults are left out.
    package static func write(_ plan: Plan) -> String {
        var out = [firstLine]
        for avenue in plan.avenues {
            out += ["", "[[avenue]]"]
            out += lines(
                0, [("key", avenue.key), ("name", avenue.name), ("color", avenue.color), ("symbol", avenue.symbol)])
        }
        for label in plan.labels {
            out += ["", "[[label]]"]
            out += lines(0, [("name", label.name), ("color", label.color)])
        }
        for project in plan.projects {
            out += ["", "[[project]]"]
            out += lines(0, [("name", project.name), ("avenue", project.avenue)])
            if !project.details.isEmpty { out.append(assignment(0, "details", project.details)) }
            if let due = project.due { out.append(assignment(0, "due", due)) }
            for task in project.tasks { out += write(task, header: "project.task", depth: 1) }
        }
        for task in plan.tasks { out += write(task, header: "task", depth: 0) }
        return out.joined(separator: "\n") + "\n"
    }

    private static func write(_ task: Plan.TaskDraft, header: String, depth: Int) -> [String] {
        let indent = String(repeating: "  ", count: depth)
        var out = ["", "\(indent)[[\(header)]]", assignment(depth, "title", task.title)]
        if let place = task.place { out.append(assignment(depth, "in", place)) }
        if let parent = task.parent { out.append(assignment(depth, "parent", parent)) }
        if !task.details.isEmpty { out.append(assignment(depth, "details", task.details)) }
        if task.status != .todo { out.append(assignment(depth, "status", statusName(task.status))) }
        if task.priority != .none { out.append(assignment(depth, "priority", task.priority.rawValue)) }
        if let start = task.start { out.append(assignment(depth, "start", start)) }
        if let due = task.due { out.append(assignment(depth, "due", due)) }
        if let dueTime = task.dueTime { out.append(assignment(depth, "due_time", dueTime)) }
        if let repeats = task.repeats { out.append(assignment(depth, "repeats", repeats)) }
        if !task.labels.isEmpty { out.append("\(indent)labels = \(array(task.labels))") }
        if !task.reminders.isEmpty { out.append("\(indent)remind = \(array(task.reminders))") }
        if !task.comments.isEmpty { out.append("\(indent)comments = \(array(task.comments))") }
        for subtask in task.subtasks { out += write(subtask, header: "\(header).subtask", depth: depth + 1) }
        return out
    }

    private static func lines(_ depth: Int, _ pairs: [(String, String)]) -> [String] {
        pairs.map { assignment(depth, $0.0, $0.1) }
    }

    private static func assignment(_ depth: Int, _ key: String, _ value: String) -> String {
        let indent = String(repeating: "  ", count: depth)
        guard value.contains("\n") else { return "\(indent)\(key) = \(quoted(value))" }
        // Multi-line strings keep their lines as they are, unindented, so
        // the text reads back unchanged.
        let body = value.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(
            of: "\"\"\"", with: "\"\"\\\"")
        return "\(indent)\(key) = \"\"\"\n\(body)\"\"\""
    }

    private static func array(_ values: [String]) -> String {
        "[" + values.map(quoted).joined(separator: ", ") + "]"
    }

    private static func quoted(_ value: String) -> String {
        var out = "\""
        for scalar in value.unicodeScalars {
            switch scalar {
            case "\"": out += "\\\""
            case "\\": out += "\\\\"
            case "\n": out += "\\n"
            case "\t": out += "\\t"
            case "\r": out += "\\r"
            case _ where scalar.value < 0x20 || scalar.value == 0x7F:
                out += String(format: "\\u%04X", scalar.value)
            default: out.unicodeScalars.append(scalar)
            }
        }
        return out + "\""
    }

    static func statusName(_ status: TaskStatus) -> String {
        switch status {
        case .todo: "todo"
        case .inProgress: "in_progress"
        case .procrastinated: "procrastinated"
        case .done: "done"
        case .wontDo: "wont_do"
        }
    }

    static let statusNames = Dictionary(uniqueKeysWithValues: TaskStatus.allCases.map { ($0, statusName($0)) })

    // MARK: - Reading

    /// Reads a plan written by `write` or by hand.
    package static func read(_ text: String) throws(PlanFileError) -> Plan {
        var parser = Parser(text)
        let root = try parser.parse()
        return try plan(from: root)
    }

    private static let avenueKeys: Set<String> = ["key", "name", "color", "symbol"]
    private static let labelKeys: Set<String> = ["name", "color"]
    private static let projectKeys: Set<String> = ["name", "avenue", "details", "due"]
    private static let taskKeys: Set<String> = [
        "title", "in", "parent", "details", "status", "priority", "start", "due", "due_time", "repeats",
        "labels", "remind", "comments",
    ]

    private static func plan(from root: Node) throws(PlanFileError) -> Plan {
        var plan = Plan()
        for node in root.children("avenue") {
            try node.check(avenueKeys, children: [])
            plan.avenues.append(
                .init(
                    key: try node.required("key"), name: try node.required("name"),
                    color: try node.string("color") ?? "blue", symbol: try node.string("symbol") ?? "circle"))
        }
        for node in root.children("label") {
            try node.check(labelKeys, children: [])
            plan.labels.append(.init(name: try node.required("name"), color: try node.string("color") ?? "gray"))
        }
        for node in root.children("project") {
            try node.check(projectKeys, children: ["task"])
            plan.projects.append(
                .init(
                    name: try node.required("name"), avenue: try node.required("avenue"),
                    details: try node.string("details") ?? "", due: try node.string("due"),
                    tasks: try node.children("task").map { (child) throws(PlanFileError) in try task(from: child) }))
        }
        plan.tasks = try root.children("task").map { (child) throws(PlanFileError) in try task(from: child) }
        return plan
    }

    private static func task(from node: Node) throws(PlanFileError) -> Plan.TaskDraft {
        let isSubtask = node.name == "subtask"
        try node.check(taskKeys, children: isSubtask ? [] : ["subtask"])
        var task = Plan.TaskDraft(title: try node.required("title"))
        task.place = try node.string("in")
        task.parent = try node.string("parent")
        task.details = try node.string("details") ?? ""
        if let status = try node.string("status") {
            guard let value = statusNames.first(where: { $0.value == status })?.key else {
                throw node.error(
                    at: "status", .badChoice(key: "status", value: status, choices: statusNames.values.sorted()))
            }
            task.status = value
        }
        if let priority = try node.string("priority") {
            guard let value = Priority(rawValue: priority) else {
                throw node.error(
                    at: "priority",
                    .badChoice(key: "priority", value: priority, choices: Priority.allCases.map(\.rawValue)))
            }
            task.priority = value
        }
        task.start = try node.string("start")
        task.due = try node.string("due")
        task.dueTime = try node.string("due_time")
        task.repeats = try node.string("repeats")
        task.labels = try node.strings("labels")
        task.reminders = try node.strings("remind")
        task.comments = try node.strings("comments")
        task.subtasks = try node.children("subtask").map { (child) throws(PlanFileError) in try self.task(from: child) }
        return task
    }
}

/// Why a plan file can't be read, with the line to look at.
package struct PlanFileError: Error, Hashable, Sendable, CustomStringConvertible {
    package var line: Int
    package var problem: Problem

    package init(line: Int, problem: Problem) {
        self.line = line
        self.problem = problem
    }

    package enum Problem: Hashable, Sendable {
        /// The text isn't TOML Lifr reads, with what was expected.
        case syntax(String)
        case unknownTable(String, suggestion: String?)
        /// A `[[child]]` with no parent table above it.
        case orphanTable(String, parent: String)
        case unknownKey(String, table: String, suggestion: String?)
        case duplicateKey(String)
        case missingKey(String, table: String)
        case notAString(key: String)
        case notAList(key: String)
        case badChoice(key: String, value: String, choices: [String])
    }

    package var description: String {
        "Line \(line): "
            + {
                switch problem {
                case .syntax(let expected):
                    return "\(expected) Check for a missing quote, bracket or =."
                case .unknownTable(let name, let suggestion):
                    return "Lifr plans have no [[\(name)]] table. "
                        + (suggestion.map { "Did you mean [[\($0)]]? " } ?? "")
                        + "Use [[avenue]], [[label]], [[project]], [[project.task]], "
                        + "[[project.task.subtask]], [[task]] or [[task.subtask]]."
                case .orphanTable(let name, let parent):
                    return "[[\(name)]] needs a [[\(parent)]] above it to belong to. Move it under one, or add the "
                        + "[[\(parent)]]."
                case .unknownKey(let key, let table, let suggestion):
                    return "[[\(table)]] has no \(key). "
                        + (suggestion.map { "Did you mean \($0)? " } ?? "")
                        + "Remove it, or rename it to one of the fields a \(table) has."
                case .duplicateKey(let key):
                    return "\(key) is set twice in the same table. Keep one."
                case .missingKey(let key, let table):
                    return "Each [[\(table)]] needs a \(key). Add one, such as \(key) = \"…\"."
                case .notAString(let key):
                    return "\(key) must be text in quotes, such as \(key) = \"…\"."
                case .notAList(let key):
                    return "\(key) must be a list of text, such as \(key) = [\"…\", \"…\"]."
                case .badChoice(let key, let value, let choices):
                    return "\(key) can't be \"\(value)\". Use one of: \(choices.joined(separator: ", "))."
                }
            }()
    }
}

// MARK: - TOML subset

private enum Value {
    case string(String)
    case strings([String])
}

/// A `[[...]]` table: its keys and its child tables, in order.
private final class Node {
    let name: String
    let path: String
    let line: Int
    var values: [String: (value: Value, line: Int)] = [:]
    var tables: [(name: String, node: Node)] = []

    init(name: String, path: String, line: Int) {
        self.name = name
        self.path = path
        self.line = line
    }

    func children(_ name: String) -> [Node] {
        tables.filter { $0.name == name }.map(\.node)
    }

    func error(at key: String, _ problem: PlanFileError.Problem) -> PlanFileError {
        PlanFileError(line: values[key]?.line ?? line, problem: problem)
    }

    /// Refuses keys and child tables this kind of table doesn't have.
    func check(_ allowed: Set<String>, children: Set<String>) throws(PlanFileError) {
        for (key, entry) in values.sorted(by: { $0.value.line < $1.value.line }) where !allowed.contains(key) {
            let suggestion = closest(to: key, in: allowed)
            throw PlanFileError(line: entry.line, problem: .unknownKey(key, table: path, suggestion: suggestion))
        }
        for table in tables where !children.contains(table.name) {
            let name = "\(path).\(table.name)"
            throw PlanFileError(
                line: table.node.line,
                problem: .unknownTable(name, suggestion: closest(to: name, in: PlanFileParserTables.all)))
        }
    }

    func string(_ key: String) throws(PlanFileError) -> String? {
        guard let entry = values[key] else { return nil }
        guard case .string(let text) = entry.value else {
            throw PlanFileError(line: entry.line, problem: .notAString(key: key))
        }
        return text
    }

    func required(_ key: String) throws(PlanFileError) -> String {
        guard let text = try string(key) else {
            throw PlanFileError(line: line, problem: .missingKey(key, table: path))
        }
        return text
    }

    func strings(_ key: String) throws(PlanFileError) -> [String] {
        guard let entry = values[key] else { return [] }
        guard case .strings(let list) = entry.value else {
            throw PlanFileError(line: entry.line, problem: .notAList(key: key))
        }
        return list
    }
}

private enum PlanFileParserTables {
    static let all: Set<String> = [
        "avenue", "label", "project", "project.task", "project.task.subtask", "task", "task.subtask",
    ]
}

/// The nearest of `candidates` to `text`, when it's close enough to be a typo.
private func closest(to text: String, in candidates: Set<String>) -> String? {
    func distance(_ source: [Character], _ target: [Character]) -> Int {
        var row = Array(0...target.count)
        for (index, sourceChar) in source.enumerated() {
            var diagonal = row[0]
            row[0] = index + 1
            for (column, targetChar) in target.enumerated() {
                let above = row[column + 1]
                row[column + 1] = min(above + 1, row[column] + 1, diagonal + (sourceChar == targetChar ? 0 : 1))
                diagonal = above
            }
        }
        return row[target.count]
    }
    let wanted = Array(text.lowercased())
    let best = candidates.map { ($0, distance(wanted, Array($0))) }.min { ($0.1, $0.0) < ($1.1, $1.0) }
    guard let best, best.1 <= max(2, wanted.count / 3) else { return nil }
    return best.0
}

private struct Parser {
    private let chars: [Character]
    private var index = 0
    private var line = 1

    init(_ text: String) {
        chars = Array(text)
    }

    mutating func parse() throws(PlanFileError) -> Node {
        let root = Node(name: "", path: "", line: 1)
        var current = root
        while true {
            skipBlankLines()
            guard let char = peek else { return root }
            if char == "[" {
                current = try header(root)
            } else {
                try keyValue(into: current)
            }
        }
    }

    private var peek: Character? { index < chars.count ? chars[index] : nil }

    private func peek(_ offset: Int) -> Character? {
        index + offset < chars.count ? chars[index + offset] : nil
    }

    private mutating func advance() -> Character? {
        guard let char = peek else { return nil }
        index += 1
        if char == "\n" { line += 1 }
        return char
    }

    private func fail(_ expected: String) -> PlanFileError {
        PlanFileError(line: line, problem: .syntax(expected))
    }

    private mutating func skipSpaces() {
        while let char = peek, char == " " || char == "\t" { index += 1 }
    }

    private mutating func skipComment() {
        if peek == "#" { while let char = peek, char != "\n" { index += 1 } }
    }

    private mutating func skipBlankLines() {
        while true {
            skipSpaces()
            skipComment()
            guard let char = peek, char == "\n" || char == "\r" else { return }
            _ = advance()
        }
    }

    /// After a value: spaces, maybe a comment, then the end of the line.
    private mutating func endOfLine() throws(PlanFileError) {
        skipSpaces()
        skipComment()
        if peek == "\r" { _ = advance() }
        guard peek == nil || peek == "\n" else { throw fail("Expected the end of the line after the value.") }
    }

    private mutating func header(_ root: Node) throws(PlanFileError) -> Node {
        let headerLine = line
        guard peek(1) == "[" else {
            throw fail("Lifr plans use [[name]] headers, with two brackets, such as [[task]].")
        }
        index += 2
        var name = ""
        while let char = peek, char != "]", char != "\n" {
            name.append(char)
            index += 1
        }
        guard peek == "]", peek(1) == "]" else { throw fail("Expected ]] to close the header.") }
        index += 2
        try endOfLine()

        let path = name.split(separator: ".").map { $0.trimmingCharacters(in: .whitespaces) }
        let joined = path.joined(separator: ".")
        guard PlanFileParserTables.all.contains(joined) else {
            throw PlanFileError(
                line: headerLine,
                problem: .unknownTable(joined, suggestion: closest(to: joined, in: PlanFileParserTables.all)))
        }
        var parent = root
        for (depth, part) in path.dropLast().enumerated() {
            guard let last = parent.children(part).last else {
                throw PlanFileError(
                    line: headerLine,
                    problem: .orphanTable(joined, parent: path[...depth].joined(separator: ".")))
            }
            parent = last
        }
        guard let leaf = path.last else { throw fail("Expected a table name between [[ and ]].") }
        let node = Node(name: leaf, path: joined, line: headerLine)
        parent.tables.append((leaf, node))
        return node
    }

    private mutating func keyValue(into node: Node) throws(PlanFileError) {
        let keyLine = line
        let key = try self.key()
        skipSpaces()
        guard advance() == "=" else { throw fail("Expected = after \(key).") }
        skipSpaces()
        let value = try self.value(key)
        try endOfLine()
        guard !node.path.isEmpty else {
            throw PlanFileError(
                line: keyLine,
                problem: .syntax("\(key) is outside any table. Put it under a [[task]] or other header."))
        }
        guard node.values[key] == nil else { throw PlanFileError(line: keyLine, problem: .duplicateKey(key)) }
        node.values[key] = (value, keyLine)
    }

    private mutating func key() throws(PlanFileError) -> String {
        if peek == "\"" { return try basicString() }
        var key = ""
        while let char = peek, char.isLetter || char.isNumber || char == "_" || char == "-" {
            key.append(char)
            index += 1
        }
        guard !key.isEmpty else { throw fail("Expected a field name, such as title.") }
        return key
    }

    private mutating func value(_ key: String) throws(PlanFileError) -> Value {
        switch peek {
        case "\"", "'":
            return .string(try string())
        case "[":
            _ = advance()
            var items: [String] = []
            while true {
                skipBlankLines()
                if peek == "]" {
                    _ = advance()
                    return .strings(items)
                }
                guard peek == "\"" || peek == "'" else {
                    throw fail("\(key) must be a list of quoted text, such as [\"a\", \"b\"].")
                }
                items.append(try string())
                skipBlankLines()
                if peek == "," {
                    _ = advance()
                } else if peek != "]" {
                    throw fail("Expected , or ] in the list for \(key).")
                }
            }
        default:
            // A bare date, such as 2026-11-20.
            var word = ""
            while let char = peek, char.isNumber || char == "-" {
                word.append(char)
                index += 1
            }
            if word.count == 10, (try? CalendarDate(word)) != nil { return .string(word) }
            throw fail("\(key)'s value must be text in quotes, a list, or a date such as 2026-11-20.")
        }
    }

    private mutating func string() throws(PlanFileError) -> String {
        if peek == "'" {
            if peek(1) == "'", peek(2) == "'" { return try multiLine(quote: "'", escapes: false) }
            _ = advance()
            var out = ""
            while let char = peek, char != "'" {
                guard char != "\n" else { throw fail("A '…' string must end on the same line.") }
                out.append(char)
                index += 1
            }
            _ = advance()
            return out
        }
        if peek(1) == "\"", peek(2) == "\"" { return try multiLine(quote: "\"", escapes: true) }
        return try basicString()
    }

    private mutating func basicString() throws(PlanFileError) -> String {
        _ = advance()
        var out = ""
        while true {
            guard let char = advance(), char != "\n" else { throw fail("A \"…\" string must end on the same line.") }
            if char == "\"" { return out }
            if char == "\\" { out += try escape() } else { out.append(char) }
        }
    }

    private mutating func multiLine(quote: Character, escapes: Bool) throws(PlanFileError) -> String {
        index += 3
        if peek == "\r" { index += 1 }
        if peek == "\n" { _ = advance() }
        var out = ""
        while true {
            guard let char = peek else {
                throw fail("A multi-line string must end with \(String(repeating: quote, count: 3)).")
            }
            if char == quote, peek(1) == quote, peek(2) == quote {
                // Up to two quotes before the closing three belong to the text.
                var run = 0
                while peek(run) == quote, run < 5 { run += 1 }
                out += String(repeating: quote, count: run - 3)
                index += run
                return out
            }
            _ = advance()
            if escapes, char == "\\" {
                if let next = peek, next == "\n" || next == " " || next == "\r" {
                    // A backslash at the end of a line joins it to the next.
                    while let space = peek, space.isWhitespace { _ = advance() }
                } else {
                    out += try escape()
                }
            } else {
                out.append(char)
            }
        }
    }

    private mutating func escape() throws(PlanFileError) -> String {
        switch advance() {
        case "\"": return "\""
        case "\\": return "\\"
        case "n": return "\n"
        case "t": return "\t"
        case "r": return "\r"
        case "b": return "\u{8}"
        case "f": return "\u{C}"
        case let marker? where marker == "u" || marker == "U":
            let count = marker == "u" ? 4 : 8
            var hex = ""
            for _ in 0..<count {
                guard let digit = advance() else { break }
                hex.append(digit)
            }
            guard let code = UInt32(hex, radix: 16), let scalar = Unicode.Scalar(code) else {
                throw fail("\\\(marker) needs \(count) hex digits for a valid character.")
            }
            return String(Character(scalar))
        default:
            throw fail("Unknown escape after \\. Use \\\\ for a backslash, or \\\" for a quote.")
        }
    }
}
