import Foundation
import LifrCore
import LifrGlance
import SwiftUI

/// What the share sheet asks before adding a task: its title, where it
/// goes and when it's due. The task waits in the App Group until Lifr next
/// opens, since the share sheet can't open the records.
struct ShareView: View {
    /// When the task is due.
    enum Due: Hashable, CaseIterable {
        case none, today, tomorrow, nextWeek

        var name: LocalizedStringKey {
            switch self {
            case .none: "No Date"
            case .today: "Today"
            case .tomorrow: "Tomorrow"
            case .nextWeek: "Next Week"
            }
        }

        /// The day it means from `now`, as YYYY-MM-DD.
        func day(from now: Date, in calendar: Calendar) -> String? {
            let days: Int
            switch self {
            case .none: return nil
            case .today: days = 0
            case .tomorrow: days = 1
            case .nextWeek: days = 7
            }
            guard let date = calendar.date(byAdding: .day, value: days, to: now) else { return nil }
            return CalendarDate(date, in: calendar.timeZone).description
        }
    }

    /// An Avenue to put the task in, as Lifr last wrote them for the
    /// widgets.
    struct Choice: Hashable, Identifiable {
        var id: UUID
        var name: String
        var symbol: String
    }

    @State var title: String
    var details: String
    /// The shared text, when it holds a plan, to hand to Lifr as a draft.
    var plan: String?
    var avenues: [Choice]
    /// Called with the task to keep, or nil when the person cancels.
    var finish: (SharedTask?) -> Void

    @State private var avenueID: UUID?
    @State private var due = Due.none
    /// Words in the title the person chose to keep as text.
    @State private var ignored: Set<String> = []
    @Environment(\.locale) private var locale

    var body: some View {
        NavigationStack {
            Group {
                if let plan, let read = Shared.readPlan(plan) {
                    PlanForm(plan: read)
                } else {
                    taskForm
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Add to Lifr")
            #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { finish(nil) }
                }
                ToolbarItem(placement: .confirmationAction) {
                    if plan != nil {
                        Button("Review in Lifr") { add() }
                    } else {
                        Button("Add") { add() }
                            .disabled(entry.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
        }
    }

    private var taskForm: some View {
        Form {
            Section {
                TextField("Title", text: $title, axis: .vertical)
                    .lineLimit(1...4)
                    .onChange(of: title) { if title.isEmpty { ignored = [] } }
                if !details.isEmpty {
                    Text(details)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                }
            }
            Section {
                Picker("Avenue", selection: $avenueID) {
                    Label("Inbox", systemImage: "tray").tag(UUID?.none)
                    ForEach(avenues.filter { $0.id != Avenue.inboxID }) { avenue in
                        Label(avenue.name, systemImage: avenue.symbol).tag(UUID?.some(avenue.id))
                    }
                }
                // A day the title names wins over the picker, as in Quick
                // Add; keeping its words as text brings the picker back.
                if let day = entry.due, let words = entry.dueWords, let key = entry.dueKey {
                    LabeledContent("Due", value: day.readable(locale))
                    Button("Keep “\(words)” in the Title") { ignored.insert(key) }
                } else {
                    Picker("Due", selection: $due) {
                        ForEach(Due.allCases, id: \.self) { due in
                            Text(due.name).tag(due)
                        }
                    }
                }
            } footer: {
                Text("It's added when Lifr next opens.")
            }
        }
    }

    /// The title as Quick Add reads it, for the day it names.
    private var entry: SharedEntry {
        let order = DateFormatter.dateFormat(fromTemplate: "Md", options: 0, locale: locale) ?? "M/d"
        let monthFirst =
            (order.firstIndex(of: "M") ?? order.startIndex) <= (order.firstIndex(of: "d") ?? order.endIndex)
        return SharedEntry(
            title, today: CalendarDate(Date(), in: .current), ignoring: ignored, monthFirst: monthFirst)
    }

    private func add() {
        let now = Date()
        if let plan {
            // Lifr opens it as a draft to review; nothing is added until
            // the person applies it there.
            return finish(
                SharedTask(id: UUID(), title: title, details: plan, avenueID: nil, due: nil, sharedAt: now))
        }
        let entry = entry
        finish(
            SharedTask(
                id: UUID(), title: entry.title.trimmingCharacters(in: .whitespacesAndNewlines), details: details,
                avenueID: avenueID, due: entry.due?.description ?? due.day(from: now, in: .current), sharedAt: now))
    }
}

/// A plan shared from another assistant: what it holds, and that Lifr
/// shows it as a draft before anything is added.
private struct PlanForm: View {
    let plan: Plan

    var body: some View {
        Form {
            Section {
                if !plan.avenues.isEmpty {
                    Text("^[\(plan.avenues.count) Avenue](inflect: true)")
                }
                if !plan.projects.isEmpty {
                    Text("^[\(plan.projects.count) Project](inflect: true)")
                }
                Text("^[\(plan.taskCount) task](inflect: true)")
            } header: {
                Text("A Plan")
            } footer: {
                Text(
                    """
                    Lifr opens it as a draft the next time it opens, showing what it adds and which names match \
                    yours. Nothing is added until you apply it there.
                    """)
            }
        }
    }
}
