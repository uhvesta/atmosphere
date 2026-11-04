import Foundation
import LifrAPI
import LifrCore

/// What a `lifr` command line asks for.
package enum Command: Hashable, Sendable {
    case help
    case version
    /// `lifr mcp`: serve MCP over stdio.
    case mcp
    /// One request to the app.
    case request(APIMethod, JSONValue)
    /// `lifr add --plan FILE`: the plan in FILE, or standard input for -.
    case addPlan(file: String)
}

/// A command line `lifr` can't run. `message` says what to type instead.
package struct UsageError: Error, Hashable, Sendable, CustomStringConvertible {
    package var message: String

    init(_ message: String) { self.message = message }

    package var description: String { "\(message)\nRun `lifr help` for every command." }
}

/// A command line, parsed.
package struct Invocation: Hashable, Sendable {
    package var command: Command
    /// Print the response as JSON, as the API sends it.
    package var json: Bool

    package init(_ arguments: [String]) throws(UsageError) {
        var words = Words(arguments)
        json = words.flag("--json")
        guard let name = words.next() else {
            command = .help
            return
        }
        switch name {
        case "help", "--help", "-h": command = .help
        case "version", "--version": command = .version
        case "mcp": command = .mcp
        default:
            guard let parse = Self.commands[name] else {
                throw UsageError("\"\(name)\" isn't a lifr command.")
            }
            command = try parse(&words)
        }
        try words.finish(name)
    }

    private typealias Parse = (inout Words) throws(UsageError) -> Command

    private static let commands: [String: Parse] = [
        "avenues": { _ in .request(.listAvenues, [:]) },
        "projects": { words throws(UsageError) in
            var params: [String: JSONValue] = [:]
            params["avenue"] = words.next().map(JSONValue.string)
            if words.flag("--closed") { params["includeClosed"] = true }
            return .request(.listProjects, .object(params))
        },
        "search": { words throws(UsageError) in
            var params: [String: JSONValue] = [:]
            params["avenue"] = try words.option("--avenue").map(JSONValue.string)
            params["project"] = try words.option("--project").map(JSONValue.string)
            let statuses = try words.options("--status").map { (word) throws(UsageError) in try status(word) }
            if !statuses.isEmpty { params["statuses"] = .array(statuses.map { .string($0.rawValue) }) }
            if words.flag("--closed") { params["includeClosed"] = true }
            if let limit = try words.option("--limit") {
                guard let number = Int(limit), number > 0 else {
                    throw UsageError("--limit takes a number of tasks, such as --limit 20.")
                }
                params["limit"] = .number(Double(number))
            }
            let query = words.rest().joined(separator: " ")
            if !query.isEmpty { params["query"] = .string(query) }
            return .request(.searchTasks, .object(params))
        },
        "show": { words throws(UsageError) in
            .request(.getTask, ["task": .string(try words.required("a task code, such as HEALTH-7K2Q"))])
        },
        "add": { words throws(UsageError) in
            if let file = try words.option("--plan") {
                if let extra = words.rest().first {
                    throw UsageError(
                        "lifr add --plan takes only the file, not \"\(extra)\". Put every task in the plan.")
                }
                return .addPlan(file: file)
            }
            var params = try taskFields(&words)
            params["avenue"] = try words.option("--avenue").map(JSONValue.string)
            params["parent"] = try words.option("--parent").map(JSONValue.string)
            params["project"] = try words.option("--project").map(JSONValue.string)
            params["status"] = try words.option("--status").map { (word) throws(UsageError) in
                .string(try status(word).rawValue)
            }
            let title = words.rest().joined(separator: " ")
            guard !title.isEmpty else { throw UsageError("lifr add needs a title, such as lifr add Buy socks.") }
            params["title"] = .string(title)
            return .request(.createTask, .object(params))
        },
        "edit": { words throws(UsageError) in
            var params = try taskFields(&words)
            let clear = try words.options("--clear").map { (word) throws(UsageError) -> JSONValue in
                switch word {
                case "start": "startDate"
                case "due": "dueDate"
                case "time": "dueTime"
                default: throw UsageError("--clear takes start, due or time, not \"\(word)\".")
                }
            }
            if !clear.isEmpty { params["clear"] = .array(clear) }
            params["task"] = .string(try words.required("a task code, such as HEALTH-7K2Q"))
            return .request(.updateTask, .object(params))
        },
        "status": { words throws(UsageError) in
            var params: [String: JSONValue] = [:]
            if words.flag("--cascade") { params["cascade"] = true }
            params["task"] = .string(try words.required("a task code, such as HEALTH-7K2Q"))
            params["status"] = .string(try status(try words.required("a status, such as done")).rawValue)
            return .request(.setStatus, .object(params))
        },
        "move": { words throws(UsageError) in
            var params: [String: JSONValue] = [:]
            params["avenue"] = try words.option("--avenue").map(JSONValue.string)
            params["project"] = try words.option("--project").map(JSONValue.string)
            params["parent"] = try words.option("--parent").map(JSONValue.string)
            if words.flag("--top-level") { params["topLevel"] = true }
            params["task"] = .string(try words.required("a task code, such as HEALTH-7K2Q"))
            return .request(.moveTask, .object(params))
        },
        "comment": { words throws(UsageError) in
            let task = try words.required("a task code, such as HEALTH-7K2Q")
            let body = words.rest().joined(separator: " ")
            guard !body.isEmpty else {
                throw UsageError(
                    "lifr comment needs the comment after the task, such as lifr comment HEALTH-7K2Q Booked.")
            }
            return .request(.addComment, ["task": .string(task), "body": .string(body)])
        },
        "trash": { words throws(UsageError) in
            .request(.trashTask, ["task": .string(try words.required("a task code, such as HEALTH-7K2Q"))])
        },
        "unlock": { _ in .request(.unlock, [:]) },
        "export": { _ in .request(.export, [:]) },
    ]

    /// The options `add` and `edit` share.
    private static func taskFields(_ words: inout Words) throws(UsageError) -> [String: JSONValue] {
        var params: [String: JSONValue] = [:]
        params["title"] = try words.option("--title").map(JSONValue.string)
        params["details"] = try words.option("--details").map(JSONValue.string)
        params["priority"] = try words.option("--priority").map { (word) throws(UsageError) in
            guard let priority = Priority(rawValue: word) else {
                throw UsageError("--priority takes none, low, medium, high or urgent, not \"\(word)\".")
            }
            return .string(priority.rawValue)
        }
        params["startDate"] = try words.option("--start").map(JSONValue.string)
        params["dueDate"] = try words.option("--due").map(JSONValue.string)
        params["dueTime"] = try words.option("--time").map(JSONValue.string)
        let labels = try words.options("--label")
        if !labels.isEmpty { params["labels"] = .array(labels.map(JSONValue.string)) }
        return params
    }

    /// A status as people type it: `in-progress` or `inProgress`.
    private static func status(_ word: String) throws(UsageError) -> TaskStatus {
        let key = word.lowercased().replacingOccurrences(of: "-", with: "").replacingOccurrences(of: "'", with: "")
        guard let status = TaskStatus.allCases.first(where: { $0.rawValue.lowercased() == key }) else {
            throw UsageError("\"\(word)\" isn't a status. Use todo, in-progress, procrastinated, done or wont-do.")
        }
        return status
    }
}

/// The words of a command line, taken as the command reads them.
private struct Words {
    private var words: [String]

    init(_ words: [String]) { self.words = words }

    mutating func flag(_ name: String) -> Bool {
        guard let index = words.firstIndex(of: name) else { return false }
        words.remove(at: index)
        return true
    }

    mutating func option(_ name: String) throws(UsageError) -> String? {
        guard let index = words.firstIndex(of: name) else { return nil }
        guard index + 1 < words.count, !words[index + 1].hasPrefix("--") else {
            throw UsageError("\(name) needs a value after it.")
        }
        let value = words[index + 1]
        words.removeSubrange(index...index + 1)
        if words.contains(name) { throw UsageError("Give \(name) once.") }
        return value
    }

    mutating func options(_ name: String) throws(UsageError) -> [String] {
        var values: [String] = []
        while words.contains(name) {
            let index = words.firstIndex(of: name)!
            guard index + 1 < words.count, !words[index + 1].hasPrefix("--") else {
                throw UsageError("\(name) needs a value after it.")
            }
            values.append(words[index + 1])
            words.removeSubrange(index...index + 1)
        }
        return values
    }

    mutating func next() -> String? {
        guard let index = words.firstIndex(where: { !$0.hasPrefix("--") }) else { return nil }
        return words.remove(at: index)
    }

    mutating func required(_ what: String) throws(UsageError) -> String {
        guard let word = next() else { throw UsageError("Give \(what).") }
        return word
    }

    /// Every word left that isn't an option.
    mutating func rest() -> [String] {
        let rest = words.filter { !$0.hasPrefix("--") }
        words.removeAll { !$0.hasPrefix("--") }
        return rest
    }

    func finish(_ command: String) throws(UsageError) {
        guard let extra = words.first else { return }
        throw UsageError("lifr \(command) doesn't take \"\(extra)\".")
    }
}
