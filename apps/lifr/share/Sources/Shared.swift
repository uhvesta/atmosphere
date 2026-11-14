import Foundation
import LifrAPI
import LifrCore

/// What another app shared, made into a task's title and description, or
/// a plan for Lifr to open as a draft.
struct Shared: Equatable {
    var title: String
    var details: String
    /// The shared text, when it holds a plan Lifr can read, such as
    /// another assistant's reply.
    var plan: String?

    init(title: String, details: String, plan: String? = nil) {
        self.title = title
        self.details = details
        self.plan = plan
    }

    /// From the page's or item's `title`, the selected or shared `text` and
    /// the shared `url`: the title is the first of those that has words,
    /// and the description keeps the link and any text that isn't the title.
    init(title: String?, text: String?, url: URL?) {
        let title = title?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let text = text?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let firstLine =
            text.split(whereSeparator: \.isNewline).first.map { String($0).trimmingCharacters(in: .whitespaces) } ?? ""
        let link = url?.absoluteString ?? ""
        let name = !title.isEmpty ? title : !firstLine.isEmpty ? firstLine : url.map(Self.name) ?? ""
        self.title = String(name.prefix(200))
        var details: [String] = []
        if !link.isEmpty { details.append(link) }
        if !text.isEmpty, text != name, text != link { details.append(text) }
        self.details = details.joined(separator: "\n\n")
        plan = Self.readPlan(text) == nil ? nil : text
    }

    /// The plan in `text`, or nil when it holds none Lifr can read.
    static func readPlan(_ text: String) -> Plan? {
        guard text.contains("[[") else { return nil }
        do {
            let plan = try PlanPrompt.read(text)
            return plan.isEmpty ? nil : plan
        } catch {
            return nil
        }
    }

    /// A link's site and path, such as "example.com/article", for when
    /// nothing better names it.
    private static func name(_ url: URL) -> String {
        let host = url.host(percentEncoded: false).map { $0.hasPrefix("www.") ? String($0.dropFirst(4)) : $0 }
        let path = url.path(percentEncoded: false)
        return (host ?? url.absoluteString) + (path == "/" ? "" : path)
    }
}

/// A shared task's title as Quick Add reads it, for the day it names. A
/// shared task carries only a day, so times, priorities and repeats stay
/// in the title as text rather than being dropped.
struct SharedEntry: Equatable {
    /// The title without the words naming the day, when there are some.
    var title: String
    var due: CalendarDate?
    /// The words naming the day, as typed.
    var dueWords: String?
    /// What to pass in `ignoring` to read `dueWords` as plain text.
    var dueKey: String?

    init(_ line: String, today: CalendarDate, ignoring: Set<String> = [], monthFirst: Bool = true) {
        // No Avenues or labels to match: the share sheet can't open them.
        let workspace = Workspace()
        var entry = QuickEntry(line, today: today, in: workspace, ignoring: ignoring, monthFirst: monthFirst)
        let kept = entry.parts.filter { if case .due = $0.kind { false } else { true } }.map(\.key)
        if !kept.isEmpty {
            entry = QuickEntry(
                line, today: today, in: workspace, ignoring: ignoring.union(kept), monthFirst: monthFirst)
        }
        let part = entry.parts.first { if case .due = $0.kind { true } else { false } }
        guard let part, case .due(let day) = part.kind else {
            title = line
            return
        }
        title = entry.title
        due = day
        dueWords = part.text
        dueKey = part.key
    }
}
