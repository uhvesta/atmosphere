import Foundation

/// How a task repeats. Closing one occurrence, as Done or Won't Do, creates
/// the next.
package struct Recurrence: Hashable, Codable, Sendable {
    package enum Unit: String, Hashable, Codable, Sendable, CaseIterable {
        case day, week, month, year
    }

    package enum Rule: Hashable, Codable, Sendable {
        /// Every `interval` days.
        case daily
        /// On these weekdays, 1 for Sunday through 7 for Saturday, every
        /// `interval` weeks.
        case weekly(weekdays: Set<Int>)
        /// On this day of the month, or the month's last day when it's
        /// shorter, every `interval` months.
        case monthly(day: Int)
        /// On the first to fourth, or the last (-1), weekday of the month.
        case monthlyWeekday(ordinal: Int, weekday: Int)
        /// On this day every `interval` years; Feb 29 is Feb 28 in other years.
        case yearly(month: Int, day: Int)
        /// `interval` of `unit` after the day it's closed.
        case afterClosing(Unit)
    }

    package var rule: Rule
    /// 1 or more.
    package var interval: Int
    /// The same for every occurrence: the first one's ID.
    package var seriesID: UUID
    /// 0 for the first occurrence.
    package var index: Int

    package init(rule: Rule, interval: Int = 1, seriesID: UUID, index: Int = 0) {
        self.rule = rule
        self.interval = interval
        self.seriesID = seriesID
        self.index = index
    }

    /// Whether the rule can produce a day: an interval of at least one, and
    /// days, weekdays and months that exist.
    var isValid: Bool {
        guard (1...999).contains(interval) else { return false }
        return switch rule {
        case .daily, .afterClosing: true
        case .weekly(let weekdays): !weekdays.isEmpty && weekdays.allSatisfy((1...7).contains)
        case .monthly(let day): (1...31).contains(day)
        case .monthlyWeekday(let ordinal, let weekday):
            ((1...4).contains(ordinal) || ordinal == -1) && (1...7).contains(weekday)
        case .yearly(let month, let day): (1...12).contains(month) && (1...31).contains(day)
        }
    }

    /// The next occurrence's due day, when this one, due `due`, is closed
    /// on `today`. A schedule skips days that are already past, so an
    /// overdue daily task comes back tomorrow, not once for every missed day.
    package func nextDue(after due: CalendarDate?, closedOn today: CalendarDate) -> CalendarDate {
        if case .afterClosing(let unit) = rule {
            return Self.advance(today, by: interval, unit)
        }
        var next = occurrence(after: due ?? today)
        // Bounded, in case a rule never moves past today.
        for _ in 0..<10_000 where next <= today {
            next = occurrence(after: next)
        }
        return next
    }

    /// The first day `rule` falls on from `day`, counting `day` itself.
    package static func firstDue(_ rule: Rule, every interval: Int, from day: CalendarDate) -> CalendarDate {
        switch rule {
        case .daily, .afterClosing:
            return day
        case .weekly(let weekdays):
            for ahead in 0..<7 where weekdays.contains(day.adding(days: ahead).weekday) {
                return day.adding(days: ahead)
            }
            return day
        case .monthly(let target):
            let this = Self.day(target, in: day.firstOfMonth)
            return this >= day ? this : Self.day(target, in: day.firstOfMonth(adding: 1))
        case .monthlyWeekday(let ordinal, let weekday):
            let this = Self.weekday(ordinal, weekday, in: day.firstOfMonth)
            return this >= day ? this : Self.weekday(ordinal, weekday, in: day.firstOfMonth(adding: 1))
        case .yearly(let month, let target):
            let this = Self.day(target, month: month, year: day.year)
            return this >= day ? this : Self.day(target, month: month, year: day.year + 1)
        }
    }

    /// The first day the schedule falls on after `day`.
    func occurrence(after day: CalendarDate) -> CalendarDate {
        switch rule {
        case .daily:
            return day.adding(days: interval)
        case .afterClosing(let unit):
            return Self.advance(day, by: interval, unit)
        case .weekly(let weekdays):
            // Weeks run Monday to Sunday.
            let offset = (day.weekday + 5) % 7
            let monday = day.adding(days: -offset)
            for later in (offset + 1)..<7 where weekdays.contains(monday.adding(days: later).weekday) {
                return monday.adding(days: later)
            }
            let nextMonday = monday.adding(days: 7 * interval)
            for later in 0..<7 where weekdays.contains(nextMonday.adding(days: later).weekday) {
                return nextMonday.adding(days: later)
            }
            return day.adding(days: 7 * interval)
        case .monthly(let target):
            let this = Self.day(target, in: day.firstOfMonth)
            if this > day { return this }
            return Self.day(target, in: day.firstOfMonth(adding: interval))
        case .monthlyWeekday(let ordinal, let weekday):
            let this = Self.weekday(ordinal, weekday, in: day.firstOfMonth)
            if this > day { return this }
            return Self.weekday(ordinal, weekday, in: day.firstOfMonth(adding: interval))
        case .yearly(let month, let target):
            let this = Self.day(target, month: month, year: day.year)
            if this > day { return this }
            return Self.day(target, month: month, year: day.year + interval)
        }
    }

    private static func advance(_ day: CalendarDate, by count: Int, _ unit: Unit) -> CalendarDate {
        switch unit {
        case .day: day.adding(days: count)
        case .week: day.adding(days: 7 * count)
        case .month: Self.day(day.day, in: day.firstOfMonth(adding: count))
        case .year: Self.day(day.day, month: day.month, year: day.year + count)
        }
    }

    /// `target` in the month that starts on `first`, or its last day.
    private static func day(_ target: Int, in first: CalendarDate) -> CalendarDate {
        day(target, month: first.month, year: first.year)
    }

    private static func day(_ target: Int, month: Int, year: Int) -> CalendarDate {
        let last = CalendarDate.days(inMonth: month, year: year)
        let first = (try? CalendarDate(year: year, month: month, day: 1)) ?? CalendarDate(Date(), in: .gmt)
        return first.adding(days: min(max(target, 1), last) - 1)
    }

    /// The `ordinal`th `weekday` in the month starting on `first`; -1 is
    /// the last.
    private static func weekday(_ ordinal: Int, _ weekday: Int, in first: CalendarDate) -> CalendarDate {
        if ordinal == -1 {
            let last = first.firstOfMonth(adding: 1).adding(days: -1)
            return last.adding(days: -((last.weekday - weekday + 7) % 7))
        }
        let firstMatch = first.adding(days: (weekday - first.weekday + 7) % 7)
        return firstMatch.adding(days: 7 * (ordinal - 1))
    }

    /// The ID the occurrence after this one gets, the same on every device,
    /// so two that close this one offline create one record that merges.
    var nextID: UUID { Self.id(seriesID, index + 1) }

    /// An ID hashed from `base` and `salt`, the same on every device. A
    /// hash rather than XOR, so mixing an already mixed ID never undoes it.
    static func id(_ base: UUID, _ salt: Int) -> UUID {
        var input = withUnsafeBytes(of: base.uuid) { Array($0) }
        withUnsafeBytes(of: Int64(salt).littleEndian) { input += $0 }
        // FNV-1a, run with two seeds for 16 bytes.
        func fnv(_ seed: UInt64) -> UInt64 {
            var hash = seed
            for byte in input {
                hash ^= UInt64(byte)
                hash = hash &* 0x0000_0100_0000_01B3
            }
            return hash
        }
        var bytes = [UInt8]()
        for seed: UInt64 in [0xCBF2_9CE4_8422_2325, 0x84222325_CBF29CE4] {
            withUnsafeBytes(of: fnv(seed).littleEndian) { bytes += $0 }
        }
        // A version 4, RFC 4122 UUID, like the random ones.
        bytes[6] = (bytes[6] & 0x0F) | 0x40
        bytes[8] = (bytes[8] & 0x3F) | 0x80
        return UUID(
            uuid: (
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9],
                bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
            ))
    }
}

extension Recurrence: CustomStringConvertible {
    /// Such as "Every 2 weeks on Mon, Thu" or "3 days after closing".
    package var description: String { Self.describing(rule, every: interval) }

    /// Such as "Every week on Mon", for a repeat not yet on a task.
    package static func describing(_ rule: Rule, every interval: Int) -> String {
        let symbols = CalendarDate.utcCalendar.shortWeekdaySymbols
        func name(_ weekday: Int) -> String { symbols.indices.contains(weekday - 1) ? symbols[weekday - 1] : "" }
        let every = interval == 1 ? "Every" : "Every \(interval)"
        switch rule {
        case .daily:
            return interval == 1 ? "Every day" : "Every \(interval) days"
        case .weekly(let weekdays):
            if weekdays == [2, 3, 4, 5, 6], interval == 1 { return "Every weekday" }
            // Monday first.
            let days = weekdays.sorted { ($0 + 5) % 7 < ($1 + 5) % 7 }.map(name).joined(separator: ", ")
            return interval == 1 ? "Every week on \(days)" : "\(every) weeks on \(days)"
        case .monthly(let day):
            return (interval == 1 ? "Every month" : "\(every) months") + " on day \(day)"
        case .monthlyWeekday(let ordinal, let weekday):
            let which = ["last", "", "first", "second", "third", "fourth"][ordinal + 1]
            return (interval == 1 ? "Every month" : "\(every) months") + " on the \(which) \(name(weekday))"
        case .yearly(let month, let day):
            let monthName = CalendarDate.utcCalendar.shortMonthSymbols[month - 1]
            return (interval == 1 ? "Every year" : "\(every) years") + " on \(monthName) \(day)"
        case .afterClosing(let unit):
            let units = interval == 1 ? unit.rawValue : "\(interval) \(unit.rawValue)s"
            return "\(interval == 1 ? "1 " : "")\(units) after closing"
        }
    }
}

extension Workspace {
    /// Makes a top-level task repeat, or stop with `nil`. A task that
    /// already repeats keeps its place in its series.
    package mutating func setRecurrence(
        _ rule: Recurrence.Rule?, every interval: Int = 1, of id: UUID, in context: Context
    ) throws(LifrError) -> Change {
        let task = try liveTask(id)
        guard !task.isSubtask else { throw .subtaskCannotRepeat(id) }
        let recurrence = rule.map {
            Recurrence(
                rule: $0, interval: interval, seriesID: task.recurrence?.seriesID ?? task.id,
                index: task.recurrence?.index ?? 0)
        }
        if let recurrence, !recurrence.isValid { throw .invalidRecurrence }
        var change = Change()
        guard task.recurrence != recurrence else { return change }
        update(id, in: context, &change) { $0.recurrence = recurrence }
        record(
            .fieldChanged, on: id, field: .recurrence, oldValue: task.recurrence?.description,
            newValue: recurrence?.description, by: context.actor, in: context, &change)
        return change
    }

    /// Creates the occurrence after `task`, which was just closed, unless
    /// one already exists: Sub tasks open again, reminders carried over, and
    /// its days moved to the next one on the schedule.
    mutating func createNextOccurrence(after task: LifrTask, in context: Context, _ change: inout Change) {
        guard let recurrence = task.recurrence, task.parentID == nil, tasks[recurrence.nextID] == nil else { return }
        let today = CalendarDate(context.now, in: context.timeZone)
        let anchor = task.dueDate ?? task.startDate
        let next = recurrence.nextDue(after: anchor, closedOn: today)
        let shift = anchor.map { $0.days(until: next) } ?? 0
        var dueDate: CalendarDate?
        var startDate: CalendarDate?
        if task.dueDate != nil {
            dueDate = next
            startDate = task.startDate?.adding(days: shift)
        } else if task.startDate != nil {
            startDate = next
        } else {
            dueDate = next
        }
        let projectID = task.projectID.flatMap { projects[$0] }?.id
        let id = recurrence.nextID
        let siblings = tasks.values.filter { $0.parentID == nil && $0.avenueID == task.avenueID }
        var successor = task
        successor.id = id
        successor.code = seriesCode(for: id)
        successor.status = .todo
        successor.startDate = startDate
        successor.dueDate = dueDate
        successor.dueTime = dueDate == nil ? nil : task.dueTime
        successor.projectID = projectID
        successor.sortOrder = SortKey.between(siblings.map(\.sortOrder).max(), nil)
        successor.createdAt = context.now
        successor.updatedAt = context.now
        successor.lastActivityAt = context.now
        successor.closedAt = nil
        successor.deletedAt = nil
        successor.recurrence?.index = recurrence.index + 1
        add(successor, in: context, &change)

        let subtasks = tasks.values.filter { $0.parentID == task.id && $0.deletedAt == nil }
            .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
        for (offset, subtask) in subtasks.enumerated() {
            var copy = subtask
            copy.id = Recurrence.id(id, offset + 1)
            copy.code = seriesCode(for: copy.id)
            copy.status = .todo
            copy.parentID = id
            copy.projectID = projectID
            copy.startDate = subtask.startDate?.adding(days: shift)
            copy.dueDate = subtask.dueDate?.adding(days: shift)
            copy.dueTime = copy.dueDate == nil ? nil : subtask.dueTime
            copy.createdAt = context.now
            copy.updatedAt = context.now
            copy.lastActivityAt = context.now
            copy.closedAt = nil
            add(copy, in: context, &change)
            record(
                .subtaskAdded, on: id, newValue: copy.code.rawValue, reason: .nextOccurrence, by: context.system,
                in: context, &change)
        }

        // A snooze rang for this occurrence only.
        let carried = reminders.values.filter { $0.taskID == task.id && !$0.isOneOff }
            .sorted { $0.id.uuidString < $1.id.uuidString }
        for (offset, reminder) in carried.enumerated() {
            let trigger: Reminder.Trigger
            switch reminder.trigger {
            case .beforeDue:
                guard dueDate != nil else { continue }
                trigger = reminder.trigger
            case .at(let date):
                // Calendar days, so it rings at the same clock time across
                // a change to or from summer time.
                var calendar = Calendar(identifier: .gregorian)
                calendar.timeZone = context.timeZone
                let moved = calendar.date(byAdding: .day, value: shift, to: date)
                trigger = .at(moved ?? date.addingTimeInterval(TimeInterval(shift) * 86_400))
            }
            // Clear of the Sub tasks' IDs.
            let copy = Reminder(id: Recurrence.id(id, 10_000 + offset), taskID: id, trigger: trigger)
            reminders[copy.id] = copy
            change.added.append(.reminder(copy.id))
        }
    }

    private mutating func add(_ task: LifrTask, in context: Context, _ change: inout Change) {
        change.previous[task.id] = .some(nil)
        tasks[task.id] = task
        record(
            .created, on: task.id, newValue: task.code.rawValue, reason: .nextOccurrence, by: context.system,
            in: context, &change)
    }

    /// A code read from `id`, so every device gives an occurrence the same
    /// one; the next free one when it's taken.
    private func seriesCode(for id: UUID) -> TaskCode {
        let used = Set(tasks.values.map(\.code))
        let count = TaskCode.alphabet.count
        for attempt in 0..<1_000 {
            let bytes = Recurrence.id(id, attempt).uuid
            let digits = withUnsafeBytes(of: bytes) { raw in
                (0..<TaskCode.length).map { Int(raw[$0]) % count }
            }
            let code = TaskCode(digits: digits)
            if !used.contains(code) { return code }
        }
        var generator = SystemRandomNumberGenerator()
        return TaskCode.random(using: &generator)
    }
}

extension Recurrence {
    /// Reads how a task repeats, as `description` writes it, in any case:
    /// "every day", "every 2 weeks on Mon, Thu", "every weekday", "every
    /// month on day 15", "every month on the last Fri", "every year on Mar
    /// 5", "3 days after closing", and "daily", "weekly", "monthly" and
    /// "yearly". A week, month or year with no day takes `due`'s.
    package static func resolving(_ text: String, due: CalendarDate) -> (rule: Rule, interval: Int)? {
        var words = text.lowercased().replacingOccurrences(of: ",", with: " ")
            .split(whereSeparator: \.isWhitespace).map(String.init)
            .filter { $0 != "the" && $0 != "and" }
        let aliases = ["daily": "day", "weekly": "week", "monthly": "month", "yearly": "year", "annually": "year"]
        if let first = words.first, let unit = aliases[first] { words = ["every", unit] + words.dropFirst() }
        let read: (rule: Rule, interval: Int)?
        if words.suffix(2) == ["after", "closing"] {
            read = afterClosing(Array(words.dropLast(2)))
        } else if words.first == "every" {
            read = every(Array(words.dropFirst()), due: due)
        } else {
            read = nil
        }
        guard let read, Recurrence(rule: read.rule, interval: read.interval, seriesID: UUID()).isValid else {
            return nil
        }
        return read
    }

    /// "3 days", "a week" or "month", before "after closing".
    private static func afterClosing(_ words: [String]) -> (rule: Rule, interval: Int)? {
        var words = words[...]
        var interval = 1
        if let first = words.first, let count = Int(first) {
            interval = count
            words = words.dropFirst()
        } else if words.first == "a" || words.first == "an" {
            words = words.dropFirst()
        }
        guard words.count == 1, let unit = words.first.flatMap(Self.unit) else { return nil }
        return (.afterClosing(unit), interval)
    }

    /// What follows "every".
    private static func every(_ words: [String], due: CalendarDate) -> (rule: Rule, interval: Int)? {
        var words = words[...]
        var interval = 1
        if let first = words.first, let count = Int(first) {
            interval = count
            words = words.dropFirst()
        }
        guard let first = words.first else { return nil }
        words = words.dropFirst()
        if words.first == "on" { words = words.dropFirst() }
        let rest = Array(words)
        if first == "weekday" || first == "weekdays" {
            return interval == 1 && rest.isEmpty ? (.weekly(weekdays: [2, 3, 4, 5, 6]), 1) : nil
        }
        // "every monday" and "every mon, thu" are every week on those days.
        if let day = weekday(first) {
            let days = rest.map(weekday)
            guard !days.contains(nil) else { return nil }
            return (.weekly(weekdays: Set([day] + days.compactMap { $0 })), interval)
        }
        switch unit(first) {
        case .day:
            return rest.isEmpty ? (.daily, interval) : nil
        case .week:
            if rest.isEmpty { return (.weekly(weekdays: [due.weekday]), interval) }
            let days = rest.map(weekday)
            guard !days.contains(nil) else { return nil }
            return (.weekly(weekdays: Set(days.compactMap { $0 })), interval)
        case .month:
            if rest.isEmpty { return (.monthly(day: due.day), interval) }
            if rest.count == 2, rest[0] == "day", let day = Int(rest[1]) { return (.monthly(day: day), interval) }
            if rest.count == 1, let day = dayNumber(rest[0]) { return (.monthly(day: day), interval) }
            let ordinals = ["first": 1, "1st": 1, "second": 2, "2nd": 2, "third": 3, "3rd": 3, "fourth": 4, "4th": 4]
            if rest.count == 2, let ordinal = ordinals[rest[0]] ?? (rest[0] == "last" ? -1 : nil),
                let day = weekday(rest[1])
            {
                return (.monthlyWeekday(ordinal: ordinal, weekday: day), interval)
            }
            return nil
        case .year:
            if rest.isEmpty { return (.yearly(month: due.month, day: due.day), interval) }
            guard rest.count == 2, let month = month(rest[0]), let day = dayNumber(rest[1]) else { return nil }
            return (.yearly(month: month, day: day), interval)
        case nil:
            return nil
        }
    }

    private static func unit(_ word: String) -> Unit? {
        Unit(rawValue: word.hasSuffix("s") ? String(word.dropLast()) : word)
    }

    /// 1 for Sunday through 7 for Saturday, from its name or the first
    /// three letters of it, or its plural.
    private static func weekday(_ word: String) -> Int? {
        let names = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"]
        let word = word.hasSuffix("days") ? String(word.dropLast()) : word
        guard word.count >= 3 else { return nil }
        return names.firstIndex { $0.hasPrefix(word) }.map { $0 + 1 }
    }

    private static func month(_ word: String) -> Int? {
        let names = [
            "january", "february", "march", "april", "may", "june", "july", "august", "september", "october",
            "november", "december",
        ]
        // The names `description` writes, which follow the calendar's locale.
        let written = CalendarDate.utcCalendar.shortMonthSymbols.map { $0.lowercased() }
        if let index = written.firstIndex(of: word) { return index + 1 }
        guard word.count >= 3 else { return nil }
        return names.firstIndex { $0.hasPrefix(word) }.map { $0 + 1 }
    }

    /// "15", "15th", "1st", "2nd" or "3rd".
    private static func dayNumber(_ word: String) -> Int? {
        let digits = word.prefix { $0.isNumber }
        let suffix = word.dropFirst(digits.count)
        guard !digits.isEmpty, ["", "st", "nd", "rd", "th"].contains(String(suffix)) else { return nil }
        return Int(digits)
    }
}
