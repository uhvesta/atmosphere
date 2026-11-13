import Foundation
import LifrCore

// The words the views show and read out, from the app's String Catalog
// (//apps/lifr/app:strings).

extension TaskStatus {
    /// The order a status picker lists statuses in.
    package static let pickOrder: [TaskStatus] = [.done, .inProgress, .todo, .procrastinated, .wontDo]

    package var title: String {
        switch self {
        case .todo: String(localized: "To Do")
        case .inProgress: String(localized: "In Progress")
        case .procrastinated: String(localized: "Procrastinated")
        case .done: String(localized: "Done")
        case .wontDo: String(localized: "Won't Do")
        }
    }
}

extension Priority {
    /// `nil` for no priority, which shows nothing.
    package var title: String? {
        switch self {
        case .none: nil
        case .low: String(localized: "Low")
        case .medium: String(localized: "Medium")
        case .high: String(localized: "High")
        case .urgent: String(localized: "Urgent")
        }
    }
}

extension CalendarDate {
    /// Such as "Nov 1" in English.
    func formatted(_ locale: Locale) -> String {
        let calendar = CalendarDate.utcCalendar
        return noonUTC.formatted(
            Date.FormatStyle(locale: locale, calendar: calendar, timeZone: .gmt).month(.abbreviated).day())
    }
}

extension TimeOfDay {
    /// Such as "3:00 PM" or "15:00", as `locale` writes times.
    package func formatted(_ locale: Locale) -> String {
        var components = DateComponents(year: 2000, month: 1, day: 1, hour: hour, minute: minute)
        components.calendar = CalendarDate.utcCalendar
        let date = components.date ?? Date(timeIntervalSince1970: 0)
        return date.formatted(
            Date.FormatStyle(locale: locale, calendar: CalendarDate.utcCalendar, timeZone: .gmt).hour().minute())
    }
}

extension TaskRowData.Due {
    package func text(_ locale: Locale) -> String {
        func at(_ time: TimeOfDay?) -> String { time.map { " \($0.formatted(locale))" } ?? "" }
        return switch self {
        case .overdue(let date, let time): String(localized: "Overdue, \(date.formatted(locale))\(at(time))")
        case .today(let time): String(localized: "Today\(at(time))")
        case .later(let date, let time): "\(date.formatted(locale))\(at(time))"
        }
    }

    package var isOverdue: Bool {
        if case .overdue = self { true } else { false }
    }
}

extension TaskRowData {
    /// Everything but the title, in the order VoiceOver reads it.
    package func accessibilityValue(_ locale: Locale) -> String {
        var parts = [status.title]
        if let priority = priority.title { parts.append(String(localized: "\(priority) priority")) }
        if let starts { parts.append(String(localized: "Starts \(starts.formatted(locale))")) }
        if let due { parts.append(due.text(locale)) }
        if repeats { parts.append(String(localized: "Repeats")) }
        if let subtasks {
            let closed = AttributedString(
                localized: "\(subtasks.closed) of ^[\(subtasks.total) Sub task](inflect: true) closed")
            parts.append(String(closed.characters))
        }
        if !labels.isEmpty { parts.append(labels.map(\.name).formatted(.list(type: .and))) }
        parts.append(fullCode)
        return parts.joined(separator: ", ")
    }
}

extension AgentAccess {
    package var title: String {
        switch self {
        case .full: String(localized: "Read and write")
        case .readOnly: String(localized: "Read only")
        case .hidden: String(localized: "Hidden")
        }
    }
}
