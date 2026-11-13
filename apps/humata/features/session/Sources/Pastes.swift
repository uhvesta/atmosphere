import Foundation

/// Long text pasted or dropped into the box, kept out of the field as a
/// chip so what's typed around it stays easy to read; it's sent after the
/// typed text.
package struct Paste: Identifiable, Hashable, Codable, Sendable {
    package let id: String
    package let text: String
    /// How many lines it has: counted once, as a paste can be megabytes.
    package let lineCount: Int

    package init(id: String, text: String) {
        self.id = id
        self.text = text
        lineCount = Self.lines(in: text)
    }

    private enum CodingKeys: CodingKey { case id, text }

    package init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.init(
            id: try container.decode(String.self, forKey: .id), text: try container.decode(String.self, forKey: .text))
    }

    package func encode(to encoder: any Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(id, forKey: .id)
        try container.encode(text, forKey: .text)
    }

    private static func lines(in text: String) -> Int {
        text.utf8.reduce(1) { $1 == UInt8(ascii: "\n") ? $0 + 1 : $0 }
    }

    /// Text at least this long, in characters or lines, becomes a chip.
    static let characters = 1_000
    static let lines = 20

    /// "Pasted 12 KB, 340 lines".
    package var label: String {
        let size = ByteCountFormatter.string(fromByteCount: Int64(text.utf8.count), countStyle: .file)
        let count = lineCount
        return "Pasted \(size), \(count == 1 ? "1 line" : "\(count) lines")"
    }

    /// When `new` is `old` with long text put in at one place, that text
    /// and `new` without it; nil for typing, deleting and short pastes.
    package static func takeOut(old: String, new: String) -> (draft: String, pasted: String)? {
        let prefix = zip(old, new).prefix(while: { $0 == $1 }).count
        let suffix = zip(old.dropFirst(prefix).reversed(), new.dropFirst(prefix).reversed())
            .prefix(while: { $0 == $1 }).count
        let start = new.index(new.startIndex, offsetBy: prefix)
        let end = new.index(new.endIndex, offsetBy: -suffix)
        guard start < end else { return nil }
        let pasted = String(new[start..<end])
        let isLong = pasted.count >= characters || Self.lines(in: pasted) >= lines
        guard isLong else { return nil }
        var draft = new
        draft.removeSubrange(start..<end)
        return (draft, pasted)
    }

    /// The message sent: what's typed, then each paste, a blank line apart.
    static func message(draft: String, pastes: [Paste]) -> String {
        ([draft.trimmingCharacters(in: .whitespacesAndNewlines)] + pastes.map(\.text))
            .filter { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            .joined(separator: "\n\n")
    }

    /// Kept with the draft, as JSON, so a chip outlasts quitting.
    static func encode(_ pastes: [Paste]) -> String {
        guard !pastes.isEmpty, let data = try? JSONEncoder().encode(pastes) else { return "" }
        return String(bytes: data, encoding: .utf8) ?? ""
    }

    static func decode(_ saved: String) -> [Paste] {
        guard !saved.isEmpty else { return [] }
        return (try? JSONDecoder().decode([Paste].self, from: Data(saved.utf8))) ?? []
    }
}
