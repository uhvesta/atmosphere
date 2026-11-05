import Foundation

/// Takes the person's text out of a recording: each quoted string in the
/// app's state and actions becomes a placeholder, the same text always the
/// same one, so a report still shows where text matched without showing it.
struct Redactor {
    private var placeholders: [String: String] = [:]

    /// `report` as it's kept: unchanged when it includes personal text,
    /// otherwise with the recording's text replaced and no screenshots. What
    /// the person wrote about the problem is kept either way.
    static func redact(_ report: BugReport) -> BugReport {
        guard !report.includesPersonal, var recording = report.recording else { return report }
        var redactor = Redactor()
        recording.start = redactor.redact(recording.start)
        recording.steps = recording.steps.map { step in
            var step = step
            step.action = redactor.redact(step.action)
            step.changes = step.changes.map { redactor.redact($0) }
            return step
        }
        recording.screenshots = []
        var report = report
        report.recording = recording
        return report
    }

    /// `text` with every string literal replaced: `"""` blocks and raw
    /// `#"…"#` ones included. Empty strings are left as they are.
    mutating func redact(_ text: String) -> String {
        var result = ""
        var rest = text[...]
        while let quote = rest.firstIndex(of: "\"") {
            result += rest[..<quote]
            rest = rest[quote...]
            // A raw literal's closing quote is followed by as many #s as
            // came before its opening one.
            let hashes = String(result.reversed().prefix { $0 == "#" })
            let multiline = rest.hasPrefix("\"\"\"")
            let open = multiline ? 3 : 1
            let close = (multiline ? "\"\"\"" : "\"") + hashes
            let body = rest.dropFirst(open)
            let end =
                multiline || !hashes.isEmpty
                ? body.range(of: close)?.lowerBound : Self.endOfLiteral(in: body)
            guard let end, multiline || !body[..<end].contains("\n") else {
                // An unmatched quote: drop the rest of the line, it may be text.
                let line = rest.firstIndex(of: "\n") ?? rest.endIndex
                result += "\"\(placeholder(String(rest.dropFirst()[..<line])))"
                rest = rest[line...]
                continue
            }
            let literal = String(body[..<end])
            result += literal.isEmpty ? String(rest.prefix(open)) + close : "\"\(placeholder(literal))\"" + hashes
            rest = body[end...].dropFirst(close.count)
        }
        return result + rest
    }

    /// Where the plain literal `body` is in ends, skipping escaped quotes;
    /// nil when the line ends first.
    private static func endOfLiteral(in body: Substring) -> Substring.Index? {
        var index = body.startIndex
        while index < body.endIndex {
            switch body[index] {
            case "\\":
                index = body.index(after: index)
                if index < body.endIndex { index = body.index(after: index) }
            case "\"": return index
            case "\n": return nil
            default: index = body.index(after: index)
            }
        }
        return nil
    }

    private mutating func placeholder(_ text: String) -> String {
        if let existing = placeholders[text] { return existing }
        let next = "‹text \(placeholders.count + 1)›"
        placeholders[text] = next
        return next
    }
}
