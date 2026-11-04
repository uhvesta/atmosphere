import Foundation
import LifrCore

// The words the views show and read out. English only for now.

extension TaskStatus {
    package var title: String {
        switch self {
        case .todo: "To Do"
        case .inProgress: "In Progress"
        case .procrastinated: "Procrastinated"
        case .done: "Done"
        case .wontDo: "Won't Do"
        }
    }
}

extension Priority {
    /// `nil` for no priority, which shows nothing.
    package var title: String? {
        switch self {
        case .none: nil
        case .low: "Low"
        case .medium: "Medium"
        case .high: "High"
        case .urgent: "Urgent"
        }
    }
}

extension CalendarDate {
    /// Such as "Nov 1" in English.
    func formatted(_ locale: Locale) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .gmt
        let date = calendar.date(from: DateComponents(year: year, month: month, day: day))!
        return date.formatted(
            Date.FormatStyle(locale: locale, calendar: calendar, timeZone: .gmt).month(.abbreviated).day())
    }
}

extension TaskRowData.Due {
    package func text(_ locale: Locale) -> String {
        func at(_ time: TimeOfDay?) -> String { time.map { " \($0)" } ?? "" }
        return switch self {
        case .overdue(let date, let time): "Overdue, \(date.formatted(locale))\(at(time))"
        case .today(let time): "Today\(at(time))"
        case .later(let date, let time): "\(date.formatted(locale))\(at(time))"
        }
    }

    var isOverdue: Bool {
        if case .overdue = self { true } else { false }
    }
}

extension TaskRowData {
    /// Everything but the title, in the order VoiceOver reads it.
    package func accessibilityValue(_ locale: Locale) -> String {
        var parts = [status.title]
        if let priority = priority.title { parts.append("\(priority) priority") }
        if let due { parts.append(due.text(locale)) }
        if let subtasks { parts.append("\(subtasks.closed) of \(subtasks.total) Sub tasks closed") }
        parts.append(code)
        return parts.joined(separator: ", ")
    }
}
