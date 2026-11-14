import Foundation
import HumataDaemonClient

/// Numbers that are the same on every run.
struct SplitMix {
    var state: UInt64

    init(seed: UInt64) { state = seed }

    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }

    mutating func below(_ bound: Int) -> Int { Int(next() % UInt64(bound)) }

    mutating func chance(_ percent: Int) -> Bool { below(100) < percent }
}

/// A working session's history of every kind: short and pasted messages,
/// thoughts, many tools a turn (commands with small to huge output, reads,
/// searches, edits and whole-file writes, failures, permissions asked and
/// answered), replies from a line to pages of Markdown with code, tables
/// and lists, and the odd notice, failure, compaction and stop.
struct ChatMaker {
    var random: SplitMix
    private var events: [SessionEvent] = []
    private var calls = 0

    init(seed: UInt64) { random = SplitMix(seed: seed) }

    mutating func history(turns: Int) -> [SessionEvent] {
        for turn in 0..<turns { addTurn(turn) }
        return events
    }

    private mutating func add(_ kind: SessionEvent.Kind) {
        events.append(SessionEvent(id: "e\(events.count)", time: Date(timeIntervalSince1970: 0), kind: kind))
    }

    private mutating func addTurn(_ turn: Int) {
        switch random.below(10) {
        case 0:
            add(
                .userMessage(
                    "Look at this log:\n" + lines("A pasted log line \(turn) that goes on.", 150 + random.below(600))))
        case 1: add(.userMessage("Here's the plan:\n\n" + Self.markdown(turn, sections: 2)))
        default: add(.userMessage("Now fix fixture \(turn), and say why it failed."))
        }
        add(.turnStarted)
        if random.chance(70) {
            add(.reasoning(lines("Check the fixture parses, then the next one.", 3 + random.below(80))))
        }
        let tools = [0, 1, 3, 8, 15, 25][random.below(6)]
        for _ in 0..<tools { addTool() }
        if random.chance(5) { add(.notice("Started over without its earlier conversation.")) }
        if random.chance(4) { add(.compacted) }
        if random.chance(6) {
            add(.failed("The agent stopped: the model's rate limit was reached. Wait a minute and send again."))
            add(.turnEnded(aborted: true))
            return
        }
        switch random.below(4) {
        case 0: add(.assistantMessage("Done: fixture \(turn) parses, **all rows** are fine."))
        case 1: add(.assistantMessage(Self.markdown(turn, sections: 1)))
        case 2: add(.assistantMessage(Self.markdown(turn, sections: 3)))
        default: add(.assistantMessage(Self.markdown(turn, sections: 12)))
        }
        add(.usage(model: "a", inputTokens: 1_000, outputTokens: 100, premiumRequests: 1))
        add(.turnEnded(aborted: random.chance(3)))
    }

    private mutating func addTool() {
        calls += 1
        let id = "c\(calls)"
        let (tool, arguments, output): (String, [String: String], String)
        switch random.below(7) {
        case 0:
            tool = "shell"
            arguments = ["command": "swift test --filter Parser"]
            output = lines(
                "row,ok,2026-11-13,a value long enough to wrap the line", [3, 40, 400, 6_000][random.below(4)])
        case 1:
            tool = "Read"
            arguments = ["file_path": "/work/Sources/Parser\(calls % 30).swift"]
            output = lines("    let field = fields[index].trimmingCharacters(in: .whitespaces)", 20 + random.below(900))
        case 2:
            tool = "Grep"
            arguments = ["pattern": "parse\\(", "path": "/work"]
            output = lines("/work/Sources/Parser.swift:42: func parse(_ line: Substring)", 1 + random.below(120))
        case 3, 4:
            tool = "Edit"
            arguments = [
                "file_path": "/work/Sources/Parser.swift",
                "old_string": lines("    let old = value", 1 + random.below(60)),
                "new_string": lines("    let new = value.trimmed", 1 + random.below(80)),
            ]
            output = "The file was updated."
        case 5:
            tool = "Write"
            arguments = [
                "file_path": "/work/Sources/Row\(calls).swift",
                "content": lines("let row = Row()", 5 + random.below(700)),
            ]
            output = "Wrote the file."
        default:
            tool = "WebFetch"
            arguments = ["url": "https://example.com/docs/\(calls)"]
            output = Self.markdown(calls, sections: 2)
        }
        let json = (try? JSONSerialization.data(withJSONObject: arguments)).flatMap {
            String(data: $0, encoding: .utf8)
        }
        add(.toolStarted(callID: id, tool: tool, argumentsJSON: json ?? "{}"))
        if random.chance(40) {
            let approved = random.chance(90)
            add(
                .permissionRequested(
                    requestID: "p\(id)", kind: tool == "shell" ? .shell : .write, summary: "\(tool) \(id)"))
            add(.permissionCompleted(requestID: "p\(id)", approved: approved, automatic: random.chance(50)))
            if !approved {
                add(.toolCompleted(callID: id, success: false, output: "The person denied this."))
                return
            }
        }
        add(.toolCompleted(callID: id, success: !random.chance(8), output: output))
    }

    private func lines(_ line: String, _ count: Int) -> String {
        (0..<count).map { "\(line) \($0)" }.joined(separator: "\n")
    }

    /// Markdown of `sections` sections, each with a heading, a list, code
    /// and a table.
    private static func markdown(_ n: Int, sections: Int) -> String {
        (0..<sections).map { section in
            """
            ## Step \(section + 1) of fixture \(n)

            The parser now reads each row once, and a malformed row says *which* column failed, with its line. \
            That makes the failures in `Tests.swift` point at the cause instead of the symptom.

            - **Fixtures** load once per run, so the suite is quicker.
            - The `strict` flag is gone; every read is strict.
            - Dates parse as ISO 8601 only.

            ```swift
            func parse(_ line: Substring) throws(ParseError) -> Row {
                let fields = line.split(separator: ",", omittingEmptySubsequences: false)
                guard fields.count == 4 else { throw .columns(found: fields.count) }
                return Row(name: String(fields[0]), ok: fields[1] == "ok", date: String(fields[2]))
            }
            ```

            | File | Lines | Change |
            | --- | --- | --- |
            | Parser.swift | 120 | rewritten |
            | Row.swift | 40 | new field |
            | Tests.swift | 300 | more cases |

            1. Run the tests again.
            2. Look at the failures, which now name the row.
            """
        }.joined(separator: "\n\n")
    }
}
