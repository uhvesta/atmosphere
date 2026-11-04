import ComposableArchitecture
import LifrCore
import LifrStoreClient
import LifrUI
import SwiftUI

package struct TaskDetailView: View {
    @Bindable package var store: StoreOf<TaskDetailFeature>
    @Environment(\.locale) private var locale

    package init(store: StoreOf<TaskDetailFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                Text(store.row.title)
                    .font(.title2.bold())
                Picker("Status", selection: Binding(get: { store.row.status }, set: { store.send(.statusPicked($0)) }))
                {
                    ForEach(TaskStatus.allCases, id: \.self) { status in
                        Text(status.title).tag(status)
                    }
                }
                LabeledContent("Priority", value: store.row.priority.title ?? "None")
                if let due = store.row.due {
                    LabeledContent("Due", value: due.text(locale))
                }
                LabeledContent("Avenue", value: store.row.avenue)
                if let project = store.project {
                    LabeledContent("Project", value: project)
                }
            }
            if !store.details.isEmpty {
                Section("Details") {
                    Text(LocalizedStringKey(store.details))
                }
            }
            if !store.subtasks.isEmpty {
                Section("Sub tasks") {
                    ForEach(store.subtasks) { row in
                        TaskRow(row) { store.send(.subtaskToggled(row.id)) }
                    }
                }
            }
        }
        .navigationTitle(store.row.code)
        #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
        #endif
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}

#Preview {
    let workspace = Workspace.sample(now: Date(), timeZone: .current)
    let parent = workspace.tasks.values.first { !workspace.subtasks(of: $0.id).isEmpty }!
    NavigationStack {
        TaskDetailView(
            store: Store(
                initialState: TaskDetailFeature.State(
                    id: parent.id, workspace: workspace, now: Date(), timeZone: .current)!
            ) {
                TaskDetailFeature()
            } withDependencies: {
                $0.store = .inMemory(workspace)
            })
    }
}
