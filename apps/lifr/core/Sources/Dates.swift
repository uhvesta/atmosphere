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
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        year = calendar.component(.year, from: date)
        month = calendar.component(.month, from: date)
        day = calendar.component(.day, from: date)
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

    /// The day as people read it, such as "Sun, Dec 6" in English.
    package func readable(_ locale: Locale = .current) -> String {
        noonUTC.formatted(
            Date.FormatStyle(locale: locale, calendar: Self.utcCalendar, timeZone: .gmt)
                .weekday(.abbreviated).month(.abbreviated).day())
    }

    /// A Gregorian calendar in UTC, for math on `noonUTC`.
    package static var utcCalendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .gmt
        return calendar
    }

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

    /// The time `date` shows in `timeZone`, to the minute.
    package init(_ date: Date, in timeZone: TimeZone) {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        hour = calendar.component(.hour, from: date)
        minute = calendar.component(.minute, from: date)
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
