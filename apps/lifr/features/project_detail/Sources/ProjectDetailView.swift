import ComposableArchitecture
import LifrCore
import LifrUI
import SwiftUI

/// A Project's fields as a section, for the top of its task list.
package struct ProjectDetailView: View {
    @Bindable package var store: StoreOf<ProjectDetailFeature>
    @Environment(\.timeZone) private var timeZone
    @FocusState private var focused: Field?

    private enum Field { case name, details }

    package init(store: StoreOf<ProjectDetailFeature>) {
        self.store = store
    }

    package var body: some View {
        Section {
            TextField("Name", text: $store.name, prompt: Text("Name"))
                .titleField()
                .accessibilityLabel("Project name")
                .font(.title3.bold())
                .focused($focused, equals: .name)
                .onSubmit { store.send(.nameSubmitted) }
                .submitLabel(.done)
            TextField("Details", text: $store.details, axis: .vertical)
                .accessibilityLabel("Project details")
                .focused($focused, equals: .details)
                .lineLimit(1...6)
            dueDate
            Picker("Avenue", selection: Binding(get: { store.avenueID }, set: { store.send(.avenuePicked($0)) })) {
                ForEach(store.avenueChoices) { Text($0.name).tag($0.id) }
            }
            progress
            actions
        } footer: {
            if store.status == .active, store.openCount > 0 {
                Text("Complete the Project once its tasks are closed.")
            }
        }
        .onChange(of: focused) { previous, _ in
            switch previous {
            case .name: store.send(.nameSubmitted)
            case .details: store.send(.detailsSubmitted)
            case nil: break
            }
        }
        .alert($store.scope(state: \.archive, action: \.archive))
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    @ViewBuilder
    private var dueDate: some View {
        if let day = store.dueDate {
            HStack {
                DatePicker(
                    "Due",
                    selection: Binding(
                        get: { day.date(in: timeZone) },
                        set: { store.send(.dueDatePicked(CalendarDate($0, in: timeZone))) }),
                    displayedComponents: .date)
                Button("Remove due date", systemImage: "xmark.circle.fill") { store.send(.dueDatePicked(nil)) }
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
                    .foregroundStyle(.secondary)
            }
        } else {
            Button("Add a due date") { store.send(.dueDatePicked(CalendarDate(Date(), in: timeZone))) }
        }
    }

    private var progress: some View {
        LabeledContent {
            Text("\(store.taskCount - store.openCount) of \(store.taskCount) done")
        } label: {
            switch store.status {
            case .active: Text("Progress")
            case .completed: Label("Completed", systemImage: "checkmark.circle.fill")
            case .archived: Label("Archived", systemImage: "archivebox")
            }
        }
    }

    @ViewBuilder
    private var actions: some View {
        if store.status == .active {
            Button("Complete Project", systemImage: "checkmark.circle") { store.send(.completeTapped) }
                .disabled(!store.canComplete)
            Button("Archive Project", systemImage: "archivebox") { store.send(.archiveTapped) }
        } else {
            Button("Restore Project", systemImage: "arrow.uturn.backward") { store.send(.restoreTapped) }
        }
    }
}

extension CalendarDate {
    /// Midnight of this day in `timeZone`.
    fileprivate func date(in timeZone: TimeZone) -> Date {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        return calendar.date(from: DateComponents(year: year, month: month, day: day)) ?? Date()
    }
}
