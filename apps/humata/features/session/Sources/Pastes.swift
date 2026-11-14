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
    /// A sent message put back in the box to change goes in the field up
    /// to this many bytes, however many lines it has, so it can be edited;
    /// past it, typing would lag, and it goes back as a chip.
    static let putBackBytes = 20_000
    /// A chip opens to edit up to this many bytes; a longer one opens to
    /// read, as an editor that size lags.
    static let editableBytes = 200_000

    /// Whether `text` is short enough to edit in the message's field.
    package static func fitsInBox(_ text: String) -> Bool { text.utf8.count <= putBackBytes }

    /// Whether the chip opens to edit rather than only to read.
    package var isEditable: Bool { text.utf8.count <= Self.editableBytes }

    /// "Pasted 12 KB, 340 lines".
    package var label: String {
        let size = ByteCountFormatter.string(fromByteCount: Int64(text.utf8.count), countStyle: .file)
        let count = lineCount
        return "Pasted \(size), \(count == 1 ? "1 line" : "\(count) lines")"
    }

    /// When `new` is `old` with long text put in at one place, that text
    /// and `new` without it; nil for typing, deleting and short pastes.
    ///
    /// Called for every key: what the two share is found by byte, with
    /// nothing allocated, and typing ends there, as what it puts in is too
    /// short to be a paste whatever it is.
    package static func takeOut(old: String, new: String) -> (draft: String, pasted: String)? {
        var old = old
        var new = new
        var prefix = shared(&old, &new, atEndAfter: nil)
        var suffix = shared(&old, &new, atEndAfter: prefix)
        // Judged on what was put in, not on how the length changed: a paste
        // over a selection can leave the draft shorter.
        guard new.utf8.count - prefix - suffix >= fewestBytes else { return nil }
        // The bytes can part in the middle of a character, as two flags
        // share their first half: the paste takes the whole of it.
        // `index(after:)` goes on from the start of the character an index
        // is in.
        var start = new.utf8.index(new.startIndex, offsetBy: prefix)
        if start < new.endIndex {
            let whole = new.index(before: new.index(after: start))
            if whole != start {
                start = whole
                prefix = new.utf8.distance(from: new.startIndex, to: start)
                suffix = shared(&old, &new, atEndAfter: prefix)
            }
        }
        var end = new.utf8.index(new.endIndex, offsetBy: -suffix)
        if end < new.endIndex {
            let next = new.index(after: end)
            if new.index(before: next) != end { end = next }
        }
        guard start < end else { return nil }
        let pasted = String(new[start..<end])
        let isLong =
            pasted.utf8.count >= characters && pasted.count >= characters || Self.lines(in: pasted) >= lines
        guard isLong else { return nil }
        var draft = new
        draft.removeSubrange(start..<end)
        return (draft, pasted)
    }

    /// The fewest bytes put in that can be a paste: `lines` lines of
    /// nothing, less a line end at each side that a character the bytes
    /// part in can add.
    private static let fewestBytes = Paste.lines - 3

    /// How many bytes `old` and `new` share at their start or, given the
    /// bytes they share there, at their end after those.
    private static func shared(_ old: inout String, _ new: inout String, atEndAfter prefix: Int?) -> Int {
        old.withUTF8 { old in
            new.withUTF8 { new in
                let most = min(old.count, new.count)
                var count = 0
                if let prefix {
                    while count < most - prefix, old[old.count - 1 - count] == new[new.count - 1 - count] {
                        count += 1
                    }
                } else {
                    while count < most, old[count] == new[count] { count += 1 }
                }
                return count
            }
        }
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
