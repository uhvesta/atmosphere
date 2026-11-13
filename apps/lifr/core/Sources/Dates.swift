import Foundation

/// A day on the calendar, stored as YYYY-MM-DD rather than a timestamp, so a
/// task due Nov 5 stays due Nov 5 in every time zone.
package struct CalendarDate: Hashable, Comparable, Codable, Sendable, CustomStringConvertible {
    package let year: Int
    package let month: Int
    package let day: Int

    package init(year: Int, month: Int, day: Int) throws(LifrError) {
        guard (1...9999).contains(year), (1...12).contains(month),
            (1...Self.days(inMonth: month, year: year)).contains(day)
        else {
            throw .invalidDate("\(Self.pad(year, 4))-\(Self.pad(month, 2))-\(Self.pad(day, 2))")
        }
        self.year = year
        self.month = month
        self.day = day
    }

    /// Parses YYYY-MM-DD.
    package init(_ string: String) throws(LifrError) {
        let parts = string.split(separator: "-", omittingEmptySubsequences: false)
        guard parts.count == 3, parts[0].count == 4, parts[1].count == 2, parts[2].count == 2,
            parts.allSatisfy({ $0.allSatisfy(\.isKeyDigit) }),
            let year = Int(parts[0]), let month = Int(parts[1]), let day = Int(parts[2])
        else { throw .invalidDate(string) }
        do {
            try self.init(year: year, month: month, day: day)
        } catch {
            throw .invalidDate(string)
        }
    }

    /// The day `date` falls on in `timeZone`.
    package init(_ date: Date, in timeZone: TimeZone) {
        // Worked out from the seconds rather than with a `Calendar`, which
        // costs more to make than the rows that ask for a day each.
        let local = date.timeIntervalSince1970 + TimeInterval(timeZone.secondsFromGMT(for: date))
        // civil_from_days, the inverse of `noonUTC`.
        let days = Int((local / 86_400).rounded(.down)) + 719_468
        let era = (days >= 0 ? days : days - 146_096) / 146_097
        let dayOfEra = days - era * 146_097
        let yearOfEra = (dayOfEra - dayOfEra / 1_460 + dayOfEra / 36_524 - dayOfEra / 146_096) / 365
        let dayOfYear = dayOfEra - (365 * yearOfEra + yearOfEra / 4 - yearOfEra / 100)
        let shiftedMonth = (5 * dayOfYear + 2) / 153
        day = dayOfYear - (153 * shiftedMonth + 2) / 5 + 1
        month = shiftedMonth < 10 ? shiftedMonth + 3 : shiftedMonth - 9
        year = yearOfEra + era * 400 + (month <= 2 ? 1 : 0)
    }

    /// Noon UTC on this day, so the weekday and calendar math don't depend
    /// on a time zone or a daylight saving change.
    package var noonUTC: Date {
        // Days since 1970-01-01, from Howard Hinnant's days_from_civil.
        let shifted = month <= 2 ? year - 1 : year
        let era = (shifted >= 0 ? shifted : shifted - 399) / 400
        let yearOfEra = shifted - era * 400
        let dayOfYear = (153 * (month + (month > 2 ? -3 : 9)) + 2) / 5 + day - 1
        let dayOfEra = yearOfEra * 365 + yearOfEra / 4 - yearOfEra / 100 + dayOfYear
        let days = era * 146_097 + dayOfEra - 719_468
        return Date(timeIntervalSince1970: TimeInterval(days) * 86_400 + 43_200)
    }

    /// The day `days` after this one, or before it when negative: by the
    /// calendar, so a day with a daylight saving change counts once.
    package func adding(days: Int) -> CalendarDate {
        CalendarDate(noonUTC + TimeInterval(days) * 86_400, in: .gmt)
    }

    /// The day as people read it, such as "Sun, Dec 6" in English.
    package func readable(_ locale: Locale = .current) -> String {
        noonUTC.formatted(
            Date.FormatStyle(locale: locale, calendar: Self.utcCalendar, timeZone: .gmt)
                .weekday(.abbreviated).month(.abbreviated).day())
    }

    /// A Gregorian calendar in UTC, for math on `noonUTC`.
    package static let utcCalendar: Calendar = {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .gmt
        return calendar
    }()

    package var description: String {
        "\(Self.pad(year, 4))-\(Self.pad(month, 2))-\(Self.pad(day, 2))"
    }

    package static func < (lhs: Self, rhs: Self) -> Bool {
        (lhs.year, lhs.month, lhs.day) < (rhs.year, rhs.month, rhs.day)
    }

    static func days(inMonth month: Int, year: Int) -> Int {
        switch month {
        case 2: year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) ? 29 : 28
        case 4, 6, 9, 11: 30
        default: 31
        }
    }

    static func pad(_ value: Int, _ width: Int) -> String {
        let digits = String(value)
        return String(repeating: "0", count: max(0, width - digits.count)) + digits
    }

    package init(from decoder: any Decoder) throws {
        let string = try decoder.singleValueContainer().decode(String.self)
        do {
            try self.init(string)
        } catch {
            throw DecodingError.dataCorrupted(
                .init(
                    codingPath: decoder.codingPath,
                    debugDescription: error.description,
                    underlyingError: error
                ))
        }
    }

    package func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(description)
    }
}

/// A time of day, read in the device's current time zone.
package struct TimeOfDay: Hashable, Comparable, Codable, Sendable, CustomStringConvertible {
    package let hour: Int
    package let minute: Int

    /// Parses 24-hour HH:MM.
    package init(_ string: String) throws(LifrError) {
        let parts = string.split(separator: ":", omittingEmptySubsequences: false)
        guard parts.count == 2, parts.allSatisfy({ $0.count == 2 && $0.allSatisfy(\.isKeyDigit) }),
            let hour = Int(parts[0]), let minute = Int(parts[1]),
            (0..<24).contains(hour), (0..<60).contains(minute)
        else { throw .invalidTime(string) }
        self.hour = hour
        self.minute = minute
    }

    /// `hour` and `minute`, each held inside a day.
    package init(clampingHour hour: Int, minute: Int) {
        self.hour = min(max(hour, 0), 23)
        self.minute = min(max(minute, 0), 59)
    }

    /// The time `date` shows in `timeZone`, to the minute.
    package init(_ date: Date, in timeZone: TimeZone) {
        let local = date.timeIntervalSince1970 + TimeInterval(timeZone.secondsFromGMT(for: date))
        let seconds = local - (local / 86_400).rounded(.down) * 86_400
        let minutes = Int(seconds) / 60
        hour = minutes / 60
        minute = minutes % 60
    }

    package var description: String {
        "\(CalendarDate.pad(hour, 2)):\(CalendarDate.pad(minute, 2))"
    }

    package static func < (lhs: Self, rhs: Self) -> Bool {
        (lhs.hour, lhs.minute) < (rhs.hour, rhs.minute)
    }

    package init(from decoder: any Decoder) throws {
        let string = try decoder.singleValueContainer().decode(String.self)
        do {
            try self.init(string)
        } catch {
            throw DecodingError.dataCorrupted(
                .init(
                    codingPath: decoder.codingPath,
                    debugDescription: error.description,
                    underlyingError: error
                ))
        }
    }

    package func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(description)
    }
}

extension CalendarDate {
    /// The first of this day's month.
    package var firstOfMonth: CalendarDate { adding(days: 1 - day) }

    /// The first of the month `months` after this day's, or before it when
    /// negative.
    package func firstOfMonth(adding months: Int) -> CalendarDate {
        let total = year * 12 + month - 1 + months
        if total >= 0, let first = try? CalendarDate(year: total / 12, month: total % 12 + 1, day: 1) {
            return first
        }
        var first = firstOfMonth
        for _ in 0..<abs(months) {
            first = months > 0 ? first.adding(days: 32).firstOfMonth : first.adding(days: -1).firstOfMonth
        }
        return first
    }

    /// 1 for Sunday through 7 for Saturday, as `Calendar` numbers them.
    package var weekday: Int { CalendarDate.utcCalendar.component(.weekday, from: noonUTC) }

    /// How many days `other` is after this one, or before it when negative.
    package func days(until other: CalendarDate) -> Int {
        Int((other.noonUTC.timeIntervalSince(noonUTC) / 86_400).rounded())
    }

    /// Such as "November 2026", in `locale`.
    package func monthTitle(_ locale: Locale = .current) -> String {
        noonUTC.formatted(
            Date.FormatStyle(locale: locale, calendar: Self.utcCalendar, timeZone: .gmt).month(.wide).year())
    }
}
