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
    var avenues: [Choice]
    /// Called with the task to keep, or nil when the person cancels.
    var finish: (SharedTask?) -> Void

    @State private var avenueID: UUID?
    @State private var due = Due.none

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Title", text: $title, axis: .vertical)
                        .lineLimit(1...4)
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
                    Picker("Due", selection: $due) {
                        ForEach(Due.allCases, id: \.self) { due in
                            Text(due.name).tag(due)
                        }
                    }
                } footer: {
                    Text("It's added when Lifr next opens.")
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
                    Button("Add") { add() }
                        .disabled(title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
    }

    private func add() {
        let now = Date()
        finish(
            SharedTask(
                id: UUID(), title: title.trimmingCharacters(in: .whitespacesAndNewlines), details: details,
                avenueID: avenueID, due: due.day(from: now, in: .current), sharedAt: now))
    }
}
