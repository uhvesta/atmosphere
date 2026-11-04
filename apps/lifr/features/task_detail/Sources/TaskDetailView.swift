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
            fields
            if !store.details.isEmpty {
                Section("Details") {
                    Text(LocalizedStringKey(store.details))
                }
            }
            if !store.isSubtask {
                subtasks
            }
            activity
        }
        .safeAreaInset(edge: .top) {
            if let banner = store.banner {
                BannerView(banner, undo: { store.send(.bannerUndoTapped) }, dismiss: { store.send(.bannerDismissed) })
            }
        }
        .navigationTitle(store.row.code)
        #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
        #endif
        .confirmationDialog($store.scope(state: \.prompt, action: \.prompt))
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    @ViewBuilder
    private var fields: some View {
        Section {
            Text(store.row.title)
                .font(.title2.bold())
            Picker("Status", selection: Binding(get: { store.row.status }, set: { store.send(.statusPicked($0)) })) {
                ForEach(TaskStatus.allCases, id: \.self) { status in
                    Text(status.title).tag(status)
                }
            }
            Picker(
                "Priority", selection: Binding(get: { store.row.priority }, set: { store.send(.priorityPicked($0)) })
            ) {
                ForEach(Priority.allCases, id: \.self) { priority in
                    Text(priority.title ?? "None").tag(priority)
                }
            }
            if let due = store.row.due {
                LabeledContent("Due", value: due.text(locale))
            }
            LabeledContent("Created") { Text(store.createdAt, format: .dateTime.month().day().year()) }
            if let closedAt = store.closedAt {
                LabeledContent("Closed") { Text(closedAt, format: .dateTime.month().day().year()) }
            }
            LabeledContent("Avenue", value: store.row.avenue)
            if let project = store.project {
                LabeledContent("Project", value: project)
            }
            if !store.labels.isEmpty {
                LabeledContent("Labels", value: store.labels.map { "#\($0)" }.joined(separator: " "))
            }
        }
    }

    private var subtasks: some View {
        Section {
            ForEach(store.subtasks) { row in
                TaskRow(row) { store.send(.subtaskToggled(row.id)) }
            }
            TextField("Add Sub task", text: $store.newSubtaskTitle)
                .onSubmit { store.send(.addSubtaskSubmitted) }
                .submitLabel(.done)
        } header: {
            HStack {
                Text("Sub tasks")
                Spacer()
                if let progress = store.row.subtasks {
                    Text("\(progress.closed)/\(progress.total)")
                        .accessibilityLabel("\(progress.closed) of \(progress.total) closed")
                }
            }
        }
    }

    private var activity: some View {
        Section {
            Picker("Show", selection: $store.showsCommentsOnly) {
                Text("All").tag(false)
                Text("Comments").tag(true)
            }
            .pickerStyle(.segmented)
            ForEach(store.visibleTimeline) { entry in
                VStack(alignment: .leading, spacing: 2) {
                    HStack {
                        Text(entry.author).font(.caption.bold())
                        Spacer()
                        Text(entry.date, format: .dateTime.month().day().hour().minute())
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Text(
                        entry.isComment ? LocalizedStringKey(entry.text) : LocalizedStringKey(stringLiteral: entry.text)
                    )
                    .font(entry.isComment ? .body : .callout)
                    .foregroundStyle(entry.isComment ? .primary : .secondary)
                }
                .accessibilityElement(children: .combine)
            }
            HStack {
                TextField("Add a comment", text: $store.newComment, axis: .vertical)
                    .onSubmit { store.send(.commentSubmitted) }
                Button {
                    store.send(.commentSubmitted)
                } label: {
                    Image(systemName: "paperplane.fill")
                }
                .accessibilityLabel("Send comment")
                .disabled(store.newComment.isEmpty)
            }
        } header: {
            Text("Activity")
        }
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
