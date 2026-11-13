import ComposableArchitecture
import LifrGlance
import LifrWatchLink
import SwiftUI

/// Today on the Watch.
package struct WatchView: View {
    @Bindable package var store: StoreOf<WatchFeature>

    package init(store: StoreOf<WatchFeature>) {
        self.store = store
    }

    package var body: some View {
        NavigationStack {
            // Turns over at midnight from the next day the iPhone sent.
            TimelineView(.everyMinute) { context in
                content(today: store.glance?.at(context.date))
            }
            .navigationTitle("Today")
        }
        .task { await store.send(.task).finish() }
        .sheet(item: Binding(get: { store.chosen }, set: { if $0 == nil { store.send(.choiceDismissed) } })) { row in
            StatusChoices(row: row, sent: store.sent[row.id]?.status) { store.send(.statusPicked($0)) }
        }
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    @ViewBuilder private func content(today: TodayGlance?) -> some View {
        List {
            Section {
                TextField("New Task", text: $store.draft)
                    .onSubmit { store.send(.addSubmitted) }
                ForEach(store.added) { task in
                    Label(task.title, systemImage: "tray.and.arrow.up")
                        .foregroundStyle(.secondary)
                        .accessibilityLabel(Text("\(task.title), sent to the iPhone's Inbox"))
                }
            } footer: {
                Text("Goes to the Inbox. Say or write it like Quick Add: \"Call Mum tomorrow 6pm\".")
            }
            if let today {
                if today.isLocked {
                    Section {
                        Text("^[\(today.count) task](inflect: true) today")
                        Text("Lifr's lock is on. Open Lifr on the iPhone to see them.")
                            .foregroundStyle(.secondary)
                    }
                } else if today.rows.isEmpty {
                    Section {
                        Text("Nothing due today.")
                            .foregroundStyle(.secondary)
                    }
                } else {
                    Section {
                        ForEach(today.rows) { row in
                            Button {
                                store.send(.rowTapped(row.id))
                            } label: {
                                TaskRow(row: row, sent: store.sent[row.id]?.status)
                            }
                        }
                    } footer: {
                        if today.count > today.rows.count {
                            Text("^[\(today.count - today.rows.count) more](inflect: true) on the iPhone")
                        }
                    }
                }
            } else {
                Section {
                    Text("Open Lifr on the iPhone to send Today here.")
                        .foregroundStyle(.secondary)
                }
            }
        }
    }
}

/// One of Today's tasks, with a status picked here until the iPhone
/// confirms it.
private struct TaskRow: View {
    var row: TodayGlance.Row
    var sent: WatchStatus?

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Image(systemName: (sent ?? status).symbol)
                .foregroundStyle(row.isOverdue ? .red : .secondary)
            VStack(alignment: .leading, spacing: 2) {
                Text(row.title)
                    .strikethrough(sent == .done || sent == .wontDo)
                    .lineLimit(3)
                Text(detail)
                    .font(.footnote)
                    .foregroundStyle(row.isOverdue ? .red : .secondary)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityHint(Text("Shows its statuses"))
    }

    private var status: WatchStatus {
        row.isInProgress ? .inProgress : .todo
    }

    private var detail: String {
        if let sent { return String(localized: "\(sent.title), sending…") }
        if row.isOverdue { return String(localized: "Overdue") }
        if let minutes = row.dueMinutes {
            let time = Calendar.current.date(
                bySettingHour: minutes / 60, minute: minutes % 60, second: 0, of: Date())
            return time?.formatted(date: .omitted, time: .shortened) ?? row.code
        }
        return row.code
    }
}

/// The statuses a task can take. Tapping a task only opens these, so a
/// task is never closed by one tap.
private struct StatusChoices: View {
    var row: TodayGlance.Row
    var sent: WatchStatus?
    var pick: (WatchStatus) -> Void

    var body: some View {
        List {
            Section {
                ForEach(WatchStatus.allCases) { status in
                    Button {
                        pick(status)
                    } label: {
                        Label(status.title, systemImage: status.symbol)
                    }
                    .disabled(status == current)
                }
            } header: {
                Text(row.title)
                    .lineLimit(2)
            }
        }
    }

    private var current: WatchStatus {
        sent ?? (row.isInProgress ? .inProgress : .todo)
    }
}
