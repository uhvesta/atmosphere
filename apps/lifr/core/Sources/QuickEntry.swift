import Foundation

/// What a Quick Add line says beyond its title: "Call dentist tomorrow 3pm
/// !high #health" is "Call dentist", due tomorrow at 15:00, high priority,
/// labelled Health. Pure, so Quick Add, Shortcuts and the CLI read a line
/// the same way, and a test gives the same answer on every device.
package struct QuickEntry: Equatable, Sendable {
    /// One recognised run of words in the line.
    package struct Part: Equatable, Sendable {
        package enum Kind: Equatable, Sendable {
            case due(CalendarDate)
            case time(TimeOfDay)
            case priority(Priority)
            case label(UUID)
            case avenue(UUID)
            case project(avenue: UUID, project: UUID)
        }

        package var kind: Kind
        /// The words as typed, such as "tomorrow" or "at 3pm".
        package var text: String
        /// Where the words are in the line, in characters.
        package var range: Range<Int>
        /// The words lowercased, to pass in `ignoring` to read them as
        /// plain text.
        package var key: String
    }

    /// The line without the recognised words.
    package var title: String
    package var parts: [Part]

    package var dueDate: CalendarDate? {
        parts.lazy.compactMap { if case .due(let date) = $0.kind { date } else { nil } }.first
            ?? (dueTime == nil ? nil : today)
    }

    package var dueTime: TimeOfDay? {
        parts.lazy.compactMap { if case .time(let time) = $0.kind { time } else { nil } }.first
    }

    package var priority: Priority? {
        parts.lazy.compactMap { if case .priority(let priority) = $0.kind { priority } else { nil } }.first
    }

    package var labelIDs: Set<UUID> {
        Set(parts.compactMap { if case .label(let id) = $0.kind { id } else { nil } })
    }

    /// The Avenue named with `@` or `/`, or the Avenue of the Project named.
    package var avenueID: UUID? {
        parts.lazy.compactMap {
            switch $0.kind {
            case .avenue(let id), .project(let id, _): id
            default: nil
            }
        }.first
    }

    package var projectID: UUID? {
        parts.lazy.compactMap { if case .project(_, let id) = $0.kind { id } else { nil } }.first
    }

    /// The day the line was read, for a time with no day.
    private var today: CalendarDate

    /// Reads `line` as of `today`.
    /// - Parameters:
    ///   - ignoring: words, lowercased, the person turned back into plain
    ///     text, such as "tomorrow" in "Read Tomorrow and Tomorrow".
    ///   - monthFirst: whether "11/5" is Nov 5, as in the US, or May 11.
    package init(
        _ line: String, today: CalendarDate, in workspace: Workspace, ignoring: Set<String> = [],
        monthFirst: Bool = true
    ) {
        self.today = today
        let words = Self.words(line)
        let reader = Reader(
            words: words, today: today, workspace: workspace, ignoring: ignoring, monthFirst: monthFirst)
        var parts: [Part] = []
        var kept: [String] = []
        var index = 0
        while index < words.count {
            if let (kind, count) = reader.read(at: index, taken: parts.map(\.kind)) {
                let range = words[index].range.lowerBound..<words[index + count - 1].range.upperBound
                let text = words[index..<index + count].map(\.text).joined(separator: " ")
                parts.append(Part(kind: kind, text: text, range: range, key: reader.key(index, count)))
                index += count
            } else {
                kept.append(words[index].text)
                index += 1
            }
        }
        self.parts = parts
        title = kept.joined(separator: " ")
    }

    /// A word of the line and where it is.
    fileprivate struct Word {
        var text: String
        var range: Range<Int>
        /// Lowercased, without a trailing comma or full stop.
        var key: String {
            var key = text.lowercased()
            while let last = key.last, last == "," || last == "." { key.removeLast() }
            return key
        }
    }

    private static func words(_ line: String) -> [Word] {
        var words: [Word] = []
        var current = ""
        var start = 0
        for (offset, character) in line.enumerated() {
            if character.isWhitespace {
                if !current.isEmpty { words.append(Word(text: current, range: start..<offset)) }
                current = ""
            } else {
                if current.isEmpty { start = offset }
                current.append(character)
            }
        }
        if !current.isEmpty { words.append(Word(text: current, range: start..<line.count)) }
        return words
    }
}

/// Recognises one part at a time, longest first.
private struct Reader {
    let words: [QuickEntry.Word]
    let today: CalendarDate
    let workspace: Workspace
    let ignoring: Set<String>
    let monthFirst: Bool

    /// The part starting at `index` and how many words it takes, unless
    /// that kind of part was already read.
    func read(at index: Int, taken: [QuickEntry.Part.Kind]) -> (QuickEntry.Part.Kind, Int)? {
        let hasDue = taken.contains { if case .due = $0 { true } else { false } }
        let hasTime = taken.contains { if case .time = $0 { true } else { false } }
        let hasPriority = taken.contains { if case .priority = $0 { true } else { false } }
        let hasPlace = taken.contains {
            switch $0 {
            case .avenue, .project: true
            default: false
            }
        }
        var found: [(QuickEntry.Part.Kind, Int)] = []
        if !hasDue, let (date, count) = date(at: index) { found.append((.due(date), count)) }
        if !hasTime, let (time, count) = time(at: index) { found.append((.time(time), count)) }
        if !hasPriority, let priority = priority(at: index) { found.append((.priority(priority), 1)) }
        if let label = label(at: index), !taken.contains(.label(label)) { found.append((.label(label), 1)) }
        if !hasPlace, let place = place(at: index) { found.append((place, 1)) }
        let kept = found.filter { _, count in !ignoring.contains(key(index, count)) }
        return kept.max { $0.1 < $1.1 }
    }

    /// `count` words from `index`, as `Part.key` has them.
    func key(_ index: Int, _ count: Int) -> String {
        words[index..<index + count].map(\.key).joined(separator: " ")
    }

    private func key(_ index: Int) -> String? {
        words.indices.contains(index) ? words[index].key : nil
    }

    // MARK: Days

    /// A day, with an "on", "by" or "due" before it.
    private func date(at index: Int) -> (CalendarDate, Int)? {
        if let word = key(index), ["on", "by", "due"].contains(word), let (date, count) = day(at: index + 1) {
            return (date, count + 1)
        }
        return day(at: index)
    }

    private func day(at index: Int) -> (CalendarDate, Int)? {
        guard let word = key(index) else { return nil }
        switch word {
        case "today", "tonight": return (today, 1)
        case "tomorrow": return (today.adding(days: 1), 1)
        default: break
        }
        if let weekday = Self.weekdays[word] { return (next(weekday), 1) }
        let following = key(index + 1)
        if ["next", "this"].contains(word), let following {
            if let weekday = Self.weekdays[following] ?? Self.shortWeekdays[following] {
                return (next(weekday), 2)
            }
            if following == "week", word == "next" { return (next(2), 2) }
            if following == "weekend" { return (today.weekday == 7 ? today : next(7), 2) }
            if following == "month", word == "next" {
                let month = today.month == 12 ? 1 : today.month + 1
                let year = today.month == 12 ? today.year + 1 : today.year
                if let first = try? CalendarDate(year: year, month: month, day: 1) { return (first, 2) }
            }
        }
        if word == "in", let span = span(at: index + 1) { return (span.0, span.1 + 1) }
        if let (date, count) = monthAndDay(at: index) { return (date, count) }
        if let date = numeric(word) { return (date, 1) }
        return nil
    }

    /// The first `weekday` (1 is Sunday) after today.
    private func next(_ weekday: Int) -> CalendarDate {
        let ahead = (weekday - today.weekday + 7) % 7
        return today.adding(days: ahead == 0 ? 7 : ahead)
    }

    /// "3 days", "a week", "2 weeks".
    private func span(at index: Int) -> (CalendarDate, Int)? {
        guard let count = key(index).flatMap({ $0 == "a" || $0 == "an" ? 1 : Int($0) }), (1...999).contains(count),
            let unit = key(index + 1)
        else { return nil }
        switch unit {
        case "day", "days": return (today.adding(days: count), 2)
        case "week", "weeks": return (today.adding(days: count * 7), 2)
        default: return nil
        }
    }

    /// "Nov 5", "November 5th", "5 Nov", "5th of November".
    private func monthAndDay(at index: Int) -> (CalendarDate, Int)? {
        guard let word = key(index) else { return nil }
        if let month = Self.months[word], let day = key(index + 1).flatMap(Self.dayNumber),
            let date = upcoming(month: month, day: day)
        {
            return (date, 2)
        }
        if let day = Self.dayNumber(word) {
            if let month = key(index + 1).flatMap({ Self.months[$0] }), let date = upcoming(month: month, day: day) {
                return (date, 2)
            }
            if key(index + 1) == "of", let month = key(index + 2).flatMap({ Self.months[$0] }),
                let date = upcoming(month: month, day: day)
            {
                return (date, 3)
            }
        }
        return nil
    }

    /// "2026-11-05", or "11/5" and "11/5/2027" in the locale's order.
    private func numeric(_ word: String) -> CalendarDate? {
        if let date = try? CalendarDate(word) { return date }
        let numbers = word.split(separator: "/", omittingEmptySubsequences: false)
        guard (2...3).contains(numbers.count), numbers.allSatisfy({ !$0.isEmpty && $0.count <= 4 }) else { return nil }
        let values = numbers.compactMap { Int($0) }
        guard values.count == numbers.count else { return nil }
        let (month, day) = monthFirst ? (values[0], values[1]) : (values[1], values[0])
        guard values.count == 3 else { return upcoming(month: month, day: day) }
        let year = values[2] < 100 ? 2000 + values[2] : values[2]
        return try? CalendarDate(year: year, month: month, day: day)
    }

    /// `month` and `day` this year, or next year once it's passed.
    private func upcoming(month: Int, day: Int) -> CalendarDate? {
        guard let date = try? CalendarDate(year: today.year, month: month, day: day) else { return nil }
        return date >= today ? date : try? CalendarDate(year: today.year + 1, month: month, day: day)
    }

    private static func dayNumber(_ word: String) -> Int? {
        var digits = word
        for suffix in ["st", "nd", "rd", "th"] where digits.hasSuffix(suffix) {
            digits.removeLast(suffix.count)
        }
        guard digits.count <= 2, let day = Int(digits), (1...31).contains(day) else { return nil }
        return day
    }

    private static let weekdays = [
        "sunday": 1, "monday": 2, "tuesday": 3, "wednesday": 4, "thursday": 5, "friday": 6, "saturday": 7,
    ]

    /// Only after "next" or "this": "sun" and "sat" are words too.
    private static let shortWeekdays = [
        "sun": 1, "mon": 2, "tue": 3, "tues": 3, "wed": 4, "thu": 5, "thur": 5, "thurs": 5, "fri": 6, "sat": 7,
    ]

    private static let months: [String: Int] = {
        let names = [
            "january", "february", "march", "april", "may", "june", "july", "august", "september", "october",
            "november", "december",
        ]
        var months: [String: Int] = [:]
        for (index, name) in names.enumerated() {
            months[name] = index + 1
            months[String(name.prefix(3))] = index + 1
        }
        months["sept"] = 9
        return months
    }()

    // MARK: Times

    /// "3pm", "3:30 pm", "15:30", "noon", with an "at" before it.
    private func time(at index: Int) -> (TimeOfDay, Int)? {
        if key(index) == "at", let (time, count) = clock(at: index + 1) { return (time, count + 1) }
        return clock(at: index)
    }

    private func clock(at index: Int) -> (TimeOfDay, Int)? {
        guard let word = key(index) else { return nil }
        if word == "noon" { return TimeOfDay(hour: 12, minute: 0).map { ($0, 1) } }
        if let meridiem = key(index + 1), ["am", "pm"].contains(meridiem),
            let time = Self.time(word, meridiem: meridiem)
        {
            return (time, 2)
        }
        return Self.time(word, meridiem: nil).map { ($0, 1) }
    }

    /// "3pm", "3:30pm" or "15:30"; "3" only with `meridiem`.
    private static func time(_ word: String, meridiem: String?) -> TimeOfDay? {
        var text = word
        var meridiem = meridiem
        for suffix in ["am", "pm", "a.m", "p.m"] where text.hasSuffix(suffix) && meridiem == nil {
            meridiem = String(suffix.prefix(1)) + "m"
            text.removeLast(suffix.count)
        }
        let pieces = text.split(separator: ":", omittingEmptySubsequences: false)
        guard (1...2).contains(pieces.count), pieces.allSatisfy({ (1...2).contains($0.count) }),
            let hour = Int(pieces[0])
        else { return nil }
        let minute = pieces.count == 2 ? Int(pieces[1]) : 0
        guard let minute, pieces.count == 2 ? pieces[1].count == 2 : true else { return nil }
        switch meridiem {
        case "am"?:
            guard (1...12).contains(hour) else { return nil }
            return TimeOfDay(hour: hour % 12, minute: minute)
        case "pm"?:
            guard (1...12).contains(hour) else { return nil }
            return TimeOfDay(hour: hour % 12 + 12, minute: minute)
        default:
            // A bare number is a count, not a time; "15:30" is a time.
            guard pieces.count == 2 else { return nil }
            return TimeOfDay(hour: hour, minute: minute)
        }
    }

    // MARK: Priority, labels and places

    /// "!low", "!medium", "!high" or "!urgent".
    private func priority(at index: Int) -> Priority? {
        guard let word = key(index), word.hasPrefix("!") else { return nil }
        switch word.dropFirst() {
        case "low": return .low
        case "med", "medium": return .medium
        case "high": return .high
        case "urgent": return .urgent
        default: return nil
        }
    }

    /// "#errand": a label by its name, or the one label its start names.
    private func label(at index: Int) -> UUID? {
        guard let word = key(index), word.hasPrefix("#"), word.count > 1 else { return nil }
        let name = String(word.dropFirst())
        return Self.match(name, in: workspace.labels.values.map { ($0.id, $0.name) })
    }

    /// "@health" or "/marathon": an open Avenue, or else an active Project,
    /// by its name or the one its start names.
    private func place(at index: Int) -> QuickEntry.Part.Kind? {
        guard let word = key(index), let first = word.first, first == "@" || first == "/", word.count > 1 else {
            return nil
        }
        let name = String(word.dropFirst())
        let avenues = workspace.avenues.values.filter { $0.archivedAt == nil }
        if let avenue = Self.match(name, in: avenues.map { ($0.id, $0.name) }) {
            return .avenue(avenue)
        }
        let projects = workspace.projects.values.filter { project in
            project.status == .active && avenues.contains { $0.id == project.avenueID }
        }
        guard let id = Self.match(name, in: projects.map { ($0.id, $0.name) }),
            let project = workspace.projects[id]
        else { return nil }
        return .project(avenue: project.avenueID, project: id)
    }

    /// The one name equal to `typed`, ignoring case and spaces, or the one
    /// name it's the start of.
    private static func match(_ typed: String, in names: [(UUID, String)]) -> UUID? {
        func folded(_ name: String) -> String { name.lowercased().filter { !$0.isWhitespace } }
        let typed = folded(typed)
        if let exact = names.first(where: { folded($0.1) == typed }) { return exact.0 }
        let starts = names.filter { folded($0.1).hasPrefix(typed) }
        return starts.count == 1 ? starts.first?.0 : nil
    }
}

extension TimeOfDay {
    /// `hour`:`minute`, or nil when either is out of range.
    fileprivate init?(hour: Int, minute: Int) {
        guard (0..<24).contains(hour), (0..<60).contains(minute),
            let time = try? TimeOfDay("\(CalendarDate.pad(hour, 2)):\(CalendarDate.pad(minute, 2))")
        else { return nil }
        self = time
    }
}
