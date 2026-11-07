import Foundation

/// What a person searches their conversations for: words, and when the
/// messages were written, read from phrases such as "last week", "in
/// October", "yesterday", "on Friday" or "3 days ago".
public struct ConversationQuery: Hashable, Sendable {
    /// What's searched for besides when, as typed.
    public var words: [String]
    /// When the messages were written; nil when the query doesn't say.
    public var during: DateInterval?

    public init(words: [String], during: DateInterval? = nil) {
        self.words = words
        self.during = during
    }

    /// Reads `text`, taking dates from `now` in `calendar`. Only the first
    /// time it names counts; the words around it, such as "in" or "on",
    /// are left out with it.
    public init(_ text: String, now: Date, calendar: Calendar) {
        let typed = text.split(whereSeparator: \.isWhitespace).map(String.init)
        let plain = typed.map { $0.lowercased().trimmingCharacters(in: .punctuationCharacters) }
        var words: [String] = []
        var during: DateInterval?
        var index = 0
        while index < typed.count {
            if during == nil,
                let (interval, length) = Self.time(at: index, in: plain, now: now, calendar: calendar)
            {
                during = interval
                if let last = words.last, Self.leadIns.contains(last.lowercased()) { words.removeLast() }
                index += length
                continue
            }
            words.append(typed[index])
            index += 1
        }
        self.init(words: words, during: during)
    }

    /// Whether there's nothing to search for.
    public var isEmpty: Bool { words.isEmpty && during == nil }

    /// The words, as one phrase.
    public var text: String { words.joined(separator: " ") }

    /// Whether `text` has every word, ignoring case and accents.
    public func matches(_ text: String) -> Bool {
        let text = text.folded
        return !words.isEmpty && words.allSatisfy { text.contains($0.folded) }
    }

    /// Words that only say a time follows, such as "on" in "on Friday".
    private static let leadIns: Set<String> = ["in", "on", "from", "during", "since", "around"]

    private static let months = [
        "january", "february", "march", "april", "may", "june", "july", "august", "september", "october",
        "november", "december",
    ]

    /// Months that are also everyday words, read as months only after a
    /// lead-in, such as "in May".
    private static let ambiguousMonths: Set<String> = ["march", "may"]

    private static let weekdays = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"]

    private static let units: [String: Calendar.Component] = [
        "day": .day, "days": .day, "week": .weekOfYear, "weeks": .weekOfYear, "month": .month, "months": .month,
        "year": .year, "years": .year,
    ]

    /// The time named by the words from `index`, and how many words name
    /// it; nil when they don't.
    private static func time(
        at index: Int, in words: [String], now: Date, calendar: Calendar
    ) -> (DateInterval, Int)? {
        let word = words[index]
        let next = index + 1 < words.count ? words[index + 1] : nil
        let before = index > 0 ? words[index - 1] : nil
        func span(_ component: Calendar.Component, back: Int) -> DateInterval? {
            calendar.date(byAdding: component, value: -back, to: now)
                .flatMap { calendar.dateInterval(of: component, for: $0) }
        }
        switch word {
        case "today": return span(.day, back: 0).map { ($0, 1) }
        case "yesterday": return span(.day, back: 1).map { ($0, 1) }
        case "this", "last":
            guard let next else { return nil }
            let back = word == "last" ? 1 : 0
            if let unit = units[next], ["week", "month", "year"].contains(next) {
                return span(unit, back: back).map { ($0, 2) }
            }
            if let weekday = weekdays.firstIndex(of: next) {
                return day(weekday, before: now, strictly: word == "last", calendar: calendar).map { ($0, 2) }
            }
            return nil
        default: break
        }
        if let weekday = weekdays.firstIndex(of: word) {
            return day(weekday, before: now, strictly: false, calendar: calendar).map { ($0, 1) }
        }
        if let month = months.firstIndex(of: word),
            !ambiguousMonths.contains(word) || before.map(leadIns.contains) == true
        {
            return latest(month: month + 1, before: now, calendar: calendar).map { ($0, 1) }
        }
        if let count = Int(word), count >= 0, let next, let unit = units[next],
            index + 2 < words.count, words[index + 2] == "ago"
        {
            return span(unit, back: count).map { ($0, 3) }
        }
        return nil
    }

    /// The latest day that's `weekday` (0 for Sunday), on or before `now`'s
    /// day, or before it when `strictly`.
    private static func day(_ weekday: Int, before now: Date, strictly: Bool, calendar: Calendar) -> DateInterval? {
        let today = calendar.component(.weekday, from: now) - 1
        var back = (today - weekday + 7) % 7
        if back == 0, strictly { back = 7 }
        return calendar.date(byAdding: .day, value: -back, to: now).flatMap { calendar.dateInterval(of: .day, for: $0) }
    }

    /// The latest `month` (1 for January) that isn't after `now`'s.
    private static func latest(month: Int, before now: Date, calendar: Calendar) -> DateInterval? {
        let current = calendar.component(.month, from: now)
        let back = (current - month + 12) % 12
        return calendar.date(byAdding: .month, value: -back, to: now)
            .flatMap { calendar.dateInterval(of: .month, for: $0) }
    }
}

extension Conversation {
    /// Where a search found a conversation.
    public struct Match: Hashable, Sendable {
        /// The message that has what was searched for; nil when its name
        /// does.
        public var message: Message?
        /// Whether it was found by words related to what was searched for,
        /// rather than the words themselves.
        public var isRelated: Bool

        public init(message: Message?, isRelated: Bool = false) {
            self.message = message
            self.isRelated = isRelated
        }
    }

    /// Where `query` is in this conversation, on any branch: its name, or
    /// else the newest message that has every word, written when the
    /// query says. Failing that, the newest such message with one of the
    /// `related` words or phrases, whole. With only a time, the newest
    /// message from then. Nil when there's none.
    public func match(_ query: ConversationQuery, related: [String] = []) -> Match? {
        guard !query.isEmpty else { return nil }
        let written = (messages + hidden).enumerated()
            .filter { !$0.element.text.isEmpty && query.during?.contains($0.element.at) != false }
            .sorted { ($0.element.at, $0.offset) > ($1.element.at, $1.offset) }.map(\.element)
        guard !query.words.isEmpty else { return written.first.map { Match(message: $0) } }
        if query.during == nil, let name, query.matches(name) { return Match(message: nil) }
        if let message = written.first(where: { query.matches($0.text) }) { return Match(message: message) }
        let phrases = related.map(\.searchWords).filter { !$0.isEmpty }
        guard !phrases.isEmpty else { return nil }
        return written.first { message in
            let words = message.text.searchWords
            return phrases.contains { words.contains(phrase: $0) }
        }
        .map { Match(message: $0, isRelated: true) }
    }
}

extension String {
    /// Without case or accents, to compare what was typed.
    fileprivate var folded: String { folding(options: [.caseInsensitive, .diacriticInsensitive], locale: nil) }

    /// Its words, folded, with a plural's ending taken off.
    fileprivate var searchWords: [String] {
        folded.split { !$0.isLetter && !$0.isNumber }.map { word in
            word.count > 3 && word.hasSuffix("s") && !word.hasSuffix("ss") ? String(word.dropLast()) : String(word)
        }
    }
}

extension [String] {
    /// Whether `phrase`'s words are in it, together and in order.
    fileprivate func contains(phrase: [String]) -> Bool {
        guard phrase.count <= count else { return false }
        return (0...(count - phrase.count)).contains { start in
            self[start..<(start + phrase.count)].elementsEqual(phrase)
        }
    }
}
