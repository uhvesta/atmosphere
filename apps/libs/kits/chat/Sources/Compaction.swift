import Foundation

/// What the assistant wrote of a conversation up to one message, sent in
/// place of those messages so a long conversation fits the model.
public struct ChatSummary: Hashable, Codable, Sendable {
    public var text: String
    public var at: Date

    public init(text: String, at: Date) {
        self.text = text
        self.at = at
    }

    /// A model's answer as a summary: trimmed, and cut at a sentence's end
    /// to `limit` characters. Nil when it wrote nothing.
    public static func text(from answer: String, limit: Int) -> String? {
        let text = answer.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return nil }
        guard text.count > limit else { return text }
        let cut = String(text.prefix(limit))
        guard let end = cut.lastIndex(where: { ".!?".contains($0) }) else { return cut + "…" }
        return String(cut[...end])
    }
}

extension Conversation {
    /// The newest summary of the messages shown, and the message it ends
    /// at: nil when none of them is summarized.
    public var summary: (through: UUID, summary: ChatSummary)? {
        for message in messages.reversed() {
            if let summary = summaries[message.id] { return (message.id, summary) }
        }
        return nil
    }

    /// The messages shown after the newest summary: all of them without one.
    public var unsummarized: ArraySlice<Message> {
        guard let through = summary?.through, let index = messages.firstIndex(where: { $0.id == through }) else {
            return messages[...]
        }
        return messages[(index + 1)...]
    }

    /// The messages a new summary would cover: those after the newest one
    /// but the last two, which stay as they were written so the model
    /// sees the latest exchange word for word.
    public var compactable: ArraySlice<Message> { unsummarized.dropLast(Self.keptWhole) }

    /// The messages a summary leaves as they are.
    public static var keptWhole: Int { 2 }

    /// A model's answer as a conversation's name: its first line, without
    /// quotes, a "Title:" label or a final full stop, cut to 40
    /// characters. Nil when nothing is left.
    public static func name(from answer: String) -> String? {
        var line = answer.split(whereSeparator: \.isNewline).first.map(String.init) ?? ""
        if line.lowercased().hasPrefix("title:") { line = String(line.dropFirst(6)) }
        line = line.trimmingCharacters(in: .whitespaces.union(CharacterSet(charactersIn: "\"'“”‘’*.")))
        guard !line.isEmpty else { return nil }
        return line.count > 40 ? String(line.prefix(39)) + "…" : line
    }
}

extension Array where Element: ChatEntry {
    /// The newest messages that fit `budget` characters, oldest first. The
    /// last message is always kept, cut from the front when it's too long
    /// by itself.
    public func newest(fitting budget: Int) -> [Element] {
        guard var last = self.last else { return [] }
        if last.text.count > budget { last.text = String(last.text.suffix(budget)) }
        var kept = [last]
        var used = last.text.count
        for message in dropLast().reversed() {
            used += message.text.count
            guard used <= budget else { break }
            kept.append(message)
        }
        return kept.reversed()
    }
}
