// Runs a linter once and writes the two reports a rules_lint aspect needs.
//
//     lint-runner --tool-name SwiftFormat --human out.txt --machine out.sarif \
//         [--exit-code out.exit_code] [--hint "how to fix"] -- swift-format lint ...
//
// The human report is the tool's own output. The machine report is SARIF,
// parsed from that output's `path:line:column: severity: message` lines,
// which swift-format and SwiftLint both print.
//
// With --exit-code, the tool's exit status goes to that file and the runner
// succeeds, so rules_lint decides what a finding means. Without it, a failing
// tool fails the runner, which prints the findings and the hint.
//
// Only the lint aspects call it, so it parses its few flags itself instead of
// taking a dependency.

import Foundation

@main
struct LintRunner {
    var toolName = ""
    var human = ""
    var machine = ""
    var exitCode: String?
    var hint = ""
    var command: [String] = []

    static func main() {
        do {
            let runner = try LintRunner(arguments: Array(CommandLine.arguments.dropFirst()))
            exit(try runner.run())
        } catch {
            FileHandle.standardError.write(Data("lint-runner: \(error)\n".utf8))
            exit(error is UsageError ? 2 : 1)
        }
    }

    init(arguments: [String]) throws {
        var rest = arguments[...]
        while let flag = rest.popFirst() {
            if flag == "--" {
                command = Array(rest)
                break
            }
            guard let value = rest.popFirst() else {
                throw UsageError("\(flag) needs a value")
            }
            switch flag {
            case "--tool-name": toolName = value
            case "--human": human = value
            case "--machine": machine = value
            case "--exit-code": exitCode = value
            case "--hint": hint = value
            default: throw UsageError("unknown flag \(flag)")
            }
        }
        if toolName.isEmpty || human.isEmpty || machine.isEmpty || command.isEmpty {
            throw UsageError("pass --tool-name, --human, --machine, and the tool to run after `--`")
        }
    }

    /// Runs the tool, writes the reports, and returns the runner's exit status.
    func run() throws -> Int32 {
        let (status, output) = try runTool()

        let inputs = command.dropFirst().filter { !$0.hasPrefix("-") && !$0.hasPrefix("/") }
        let text = output.split(separator: "\n", omittingEmptySubsequences: false)
            .map { relativize($0, inputs: inputs) }
            .joined(separator: "\n")

        try write(text, to: human)
        try write(sarif(toolName: toolName, output: text), to: machine)

        if let exitCode {
            try write("\(status)\n", to: exitCode)
            return 0
        }
        if status != 0 {
            FileHandle.standardError.write(Data(text.utf8))
            if !hint.isEmpty {
                FileHandle.standardError.write(Data("\n\(hint)\n".utf8))
            }
        }
        return status
    }

    /// Runs the tool with stdout and stderr in one pipe, and returns its exit
    /// status and output. A tool killed by a signal counts as status 1.
    private func runTool() throws -> (Int32, String) {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: command[0])
        process.arguments = Array(command.dropFirst())
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe
        do {
            try process.run()
        } catch {
            throw RunnerError.run(tool: command[0], underlying: error)
        }
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        let status = process.terminationReason == .exit ? process.terminationStatus : 1
        // Keep output that isn't valid UTF-8, with replacement characters,
        // rather than dropping it as the failable initializer would.
        // swiftlint:disable:next optional_data_string_conversion
        return (status, String(decoding: data, as: UTF8.self))
    }
}

struct UsageError: Error, CustomStringConvertible {
    var description: String

    init(_ description: String) {
        self.description = description
    }
}

enum RunnerError: Error, CustomStringConvertible {
    case run(tool: String, underlying: Error)
    case write(path: String, underlying: Error)

    var description: String {
        switch self {
        case .run(let tool, let underlying):
            "couldn't run \(tool): \(underlying). Check that its toolchain provides it for this execution platform."
        case .write(let path, let underlying):
            "couldn't write \(path): \(underlying)"
        }
    }
}

private func write(_ contents: String, to path: String) throws {
    do {
        try contents.write(toFile: path, atomically: false, encoding: .utf8)
    } catch {
        throw RunnerError.write(path: path, underlying: error)
    }
}

/// Tools print absolute paths: into the sandbox, or, after following its
/// symlinks, into the execroot or source tree. Rewrite a line's leading path to
/// the workspace-relative input it ends with, so reports point at the source.
func relativize(_ line: Substring, inputs: [String]) -> String {
    guard line.hasPrefix("/"), let colon = line.firstIndex(of: ":") else {
        return String(line)
    }
    let path = line[..<colon]
    guard let input = inputs.first(where: { path.hasSuffix("/" + $0) }) else {
        return String(line)
    }
    return input + line[colon...]
}

/// One `path:line:column: severity: message` line of linter output.
struct Diagnostic {
    var path: String
    var line: Int
    var column: Int
    var level: String
    var rule: String?
    var message: String

    init?(_ text: Substring) {
        let parts = text.split(separator: ":", maxSplits: 4, omittingEmptySubsequences: false)
        guard parts.count == 5, let line = Int(parts[1]), let column = Int(parts[2]) else {
            return nil
        }
        let level = parts[3].trimmingCharacters(in: .whitespaces)
        guard ["error", "warning", "note"].contains(level) else {
            return nil
        }
        self.path = String(parts[0])
        self.line = line
        self.column = column
        self.level = level
        (self.rule, self.message) = splitRule(parts[4].trimmingCharacters(in: .whitespaces))
    }
}

/// Finds the rule ID: swift-format writes `[Rule] message`, SwiftLint writes
/// `message (rule_id)`.
private func splitRule(_ message: String) -> (String?, String) {
    if message.hasPrefix("["), let end = message.range(of: "] ") {
        let rule = message[message.index(after: message.startIndex)..<end.lowerBound]
        return (String(rule), String(message[end.upperBound...]))
    }
    if message.hasSuffix(")"), let open = message.range(of: " (", options: .backwards) {
        let rule = message[open.upperBound..<message.index(before: message.endIndex)]
        if !rule.isEmpty, !rule.contains(where: \.isWhitespace) {
            return (String(rule), String(message[..<open.lowerBound]))
        }
    }
    return (nil, message)
}

/// A SARIF 2.1.0 log with one result per diagnostic line in `output`.
private func sarif(toolName: String, output: String) throws -> String {
    let results: [[String: Any]] = output.split(separator: "\n").compactMap(Diagnostic.init).map { diagnostic in
        var result: [String: Any] = [
            "level": diagnostic.level,
            "message": ["text": diagnostic.message],
            "locations": [
                [
                    "physicalLocation": [
                        "artifactLocation": ["uri": diagnostic.path],
                        "region": ["startLine": diagnostic.line, "startColumn": diagnostic.column],
                    ]
                ]
            ],
        ]
        if let rule = diagnostic.rule {
            result["ruleId"] = rule
        }
        return result
    }
    let log: [String: Any] = [
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [["tool": ["driver": ["name": toolName]], "results": results]],
    ]
    let data = try JSONSerialization.data(withJSONObject: log, options: [.sortedKeys])
    // JSONSerialization writes UTF-8.
    return String(bytes: data, encoding: .utf8) ?? ""
}
