import Foundation

/// What Siri says about Today.
package struct TodaySummary: Hashable, Sendable {
    package var text: String

    package init(_ today: Shortcuts.Today) {
        var parts: [String] = []
        switch today.count {
        case 0: parts.append(String(localized: "Nothing is due today."))
        case 1: parts.append(String(localized: "1 task is due today."))
        default: parts.append(String(localized: "\(today.count) tasks are due today."))
        }
        if today.overdue > 0 {
            parts.append(String(localized: "\(today.overdue) of them overdue."))
        }
        if today.isLocked, today.count > 0 {
            parts.append(String(localized: "Open Lifr and unlock it to see them."))
        } else if !today.titles.isEmpty {
            parts.append(today.titles.formatted(.list(type: .and)) + ".")
        }
        text = parts.joined(separator: " ")
    }
}
