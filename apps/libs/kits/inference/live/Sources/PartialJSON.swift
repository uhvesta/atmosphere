import Foundation
import JSONKit

/// JSON a model is still writing, read as far as it goes: the last string
/// cut off where it stops, and every open object and array closed.
enum PartialJSON {
    /// How far into `text` it can be read, or nil before anything can.
    /// A key without its value, and a number or word cut off, are left
    /// out until they're whole.
    static func value(_ text: String) -> JSONValue? {
        var points: [(end: String.Index, closers: String)] = []
        var closers: [Character] = []
        var inString = false
        var escaped = false
        // Whether the last point is in the string being written, to move
        // on rather than add another: one point for each string read so far.
        var inStringPoint = false
        var index = text.startIndex
        while index < text.endIndex {
            let character = text[index]
            let next = text.index(after: index)
            if inString {
                if escaped {
                    escaped = false
                } else if character == "\\" {
                    // The point before an escape stays, in case it's cut off.
                    escaped = true
                    inStringPoint = false
                } else if character == "\"" {
                    inString = false
                    inStringPoint = false
                    points.append((next, String(closers.reversed())))
                } else {
                    // Cut off in the string, it reads as what's there.
                    if inStringPoint { points.removeLast() }
                    points.append((next, "\"" + String(closers.reversed())))
                    inStringPoint = true
                }
            } else {
                switch character {
                case "\"":
                    inString = true
                    points.append((next, "\"" + String(closers.reversed())))
                    inStringPoint = true
                case "{": closers.append("}")
                case "[": closers.append("]")
                case "}", "]": _ = closers.popLast()
                default: break
                }
                if "{[}]0123456789el".contains(character) {
                    points.append((next, String(closers.reversed())))
                }
            }
            index = next
        }
        // The latest that reads, trying back a few: a key, a comma or a
        // cut-off escape doesn't, the point before it does.
        for point in points.reversed().prefix(8) {
            let candidate = text[..<point.end] + point.closers
            if let value = try? JSONDecoder().decode(JSONValue.self, from: Data(candidate.utf8)) {
                return value
            }
        }
        return nil
    }
}
