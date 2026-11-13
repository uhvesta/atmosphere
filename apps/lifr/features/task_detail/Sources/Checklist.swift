import Foundation

/// A piece of a task's description: text, or a checklist item written as
/// "- [ ] item" or "- [x] item" on its own line.
enum DescriptionPart: Hashable {
    case text(String)
    case item(line: Int, title: String, checked: Bool)
}

extension String {
    /// The description split into checklist items and the text between them.
    var descriptionParts: [DescriptionPart] {
        var parts: [DescriptionPart] = []
        var text: [Substring] = []
        func flush() {
            let joined = text.joined(separator: "\n").trimmingCharacters(in: .newlines)
            if !joined.isEmpty { parts.append(.text(joined)) }
            text = []
        }
        for (index, line) in split(separator: "\n", omittingEmptySubsequences: false).enumerated() {
            if let (checked, title) = Self.checklistItem(line) {
                flush()
                parts.append(.item(line: index, title: title, checked: checked))
            } else {
                text.append(line)
            }
        }
        flush()
        return parts
    }

    /// The description with the checklist item on `line` checked or
    /// unchecked; nil when that line isn't one.
    func togglingChecklistItem(atLine line: Int) -> String? {
        var lines = split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        guard lines.indices.contains(line), let (checked, _) = Self.checklistItem(lines[line][...]),
            let box = lines[line].range(of: checked ? "[x]" : "[ ]", options: checked ? .caseInsensitive : [])
        else { return nil }
        lines[line].replaceSubrange(box, with: checked ? "[ ]" : "[x]")
        return lines.joined(separator: "\n")
    }

    /// Whether `line` is a checklist item, and if so whether it's checked
    /// and what it says.
    private static func checklistItem(_ line: Substring) -> (checked: Bool, title: String)? {
        let trimmed = line.drop { $0 == " " || $0 == "\t" }
        guard let bullet = trimmed.first, bullet == "-" || bullet == "*" || bullet == "+" else { return nil }
        let rest = trimmed.dropFirst()
        guard rest.first == " " else { return nil }
        let box = rest.dropFirst()
        guard box.count >= 3, box.first == "[", box.dropFirst(2).first == "]" else { return nil }
        let mark = box.dropFirst().first
        let checked: Bool
        switch mark {
        case " ": checked = false
        case "x", "X": checked = true
        default: return nil
        }
        let title = box.dropFirst(3).drop { $0 == " " }
        return (checked, String(title))
    }
}
