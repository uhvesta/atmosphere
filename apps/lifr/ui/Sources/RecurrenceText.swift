import Foundation
import LifrCore

extension Recurrence {
    /// How the task repeats, in `locale`'s language and names, such as
    /// "Every 2 weeks on Mon and Thu". `description` stays the English
    /// form the journal stores and reads back.
    package func title(_ locale: Locale = .current) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.locale = locale
        let symbols = calendar.shortWeekdaySymbols
        func name(_ weekday: Int) -> String { symbols.indices.contains(weekday - 1) ? symbols[weekday - 1] : "" }
        let count = interval
        switch rule {
        case .daily:
            return count == 1
                ? String(localized: "Every day", locale: locale)
                : inflected(AttributedString(localized: "Every ^[\(count) day](inflect: true)", locale: locale))
        case .weekly(let weekdays):
            if weekdays == [2, 3, 4, 5, 6], count == 1 { return String(localized: "Every weekday", locale: locale) }
            // Monday first.
            let days = weekdays.sorted { ($0 + 5) % 7 < ($1 + 5) % 7 }.map(name)
                .formatted(.list(type: .and).locale(locale))
            return count == 1
                ? String(localized: "Every week on \(days)", locale: locale)
                : inflected(
                    AttributedString(localized: "Every ^[\(count) week](inflect: true) on \(days)", locale: locale))
        case .monthly(let day):
            return count == 1
                ? String(localized: "Every month on day \(day)", locale: locale)
                : inflected(
                    AttributedString(localized: "Every ^[\(count) month](inflect: true) on day \(day)", locale: locale))
        case .monthlyWeekday(let ordinal, let weekday):
            let on = Self.on(ordinal, name(weekday), locale)
            return count == 1
                ? String(localized: "Every month \(on)", locale: locale)
                : inflected(AttributedString(localized: "Every ^[\(count) month](inflect: true) \(on)", locale: locale))
        case .yearly(let month, let day):
            // A leap year, so February 29 has a date to name.
            let date = DateComponents(calendar: CalendarDate.utcCalendar, year: 2000, month: month, day: day).date
            let named =
                date?.formatted(Date.FormatStyle(locale: locale, timeZone: .gmt).month(.abbreviated).day())
                ?? "\(month)/\(day)"
            return count == 1
                ? String(localized: "Every year on \(named)", locale: locale)
                : inflected(
                    AttributedString(localized: "Every ^[\(count) year](inflect: true) on \(named)", locale: locale))
        case .afterClosing(let unit):
            let text: AttributedString =
                switch unit {
                case .day: AttributedString(localized: "^[\(count) day](inflect: true) after closing", locale: locale)
                case .week: AttributedString(localized: "^[\(count) week](inflect: true) after closing", locale: locale)
                case .month:
                    AttributedString(localized: "^[\(count) month](inflect: true) after closing", locale: locale)
                case .year: AttributedString(localized: "^[\(count) year](inflect: true) after closing", locale: locale)
                }
            return inflected(text)
        }
    }

    /// Which of the month's `weekday`s, a whole phrase so each language
    /// can order it its own way.
    private static func on(_ ordinal: Int, _ weekday: String, _ locale: Locale) -> String {
        switch ordinal {
        case -1: String(localized: "on the last \(weekday)", locale: locale)
        case 1: String(localized: "on the first \(weekday)", locale: locale)
        case 2: String(localized: "on the second \(weekday)", locale: locale)
        case 3: String(localized: "on the third \(weekday)", locale: locale)
        default: String(localized: "on the fourth \(weekday)", locale: locale)
        }
    }
}

/// The words of an inflected string, without its attributes.
private func inflected(_ text: AttributedString) -> String { String(text.characters) }
