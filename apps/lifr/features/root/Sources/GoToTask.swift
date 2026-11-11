import ComposableArchitecture
import LifrCore
import LifrUI
import SwiftUI

/// Go to Task…: a field that finds a task by its code, such as HEALTH-7K2Q
/// or 7K2Q, or by words in its title, its Avenue or its Project.
@Reducer
package struct GoToTaskFeature {
    @ObservableState
    package struct State: Equatable {
        package var workspace: Workspace
        package var query = ""

        package init(workspace: Workspace) {
            self.workspace = workspace
        }

        /// The open tasks that match, a task with the code first, then by
        /// title. At most `limit`, so the list stays a glance.
        package var matches: [Match] {
            let words = query.split(whereSeparator: \.isWhitespace).map(String.init)
            guard !words.isEmpty else { return [] }
            let coded = (try? workspace.task(code: query.trimmingCharacters(in: .whitespaces))).flatMap {
                $0.deletedAt == nil ? $0 : nil
            }
            let found = workspace.tasks.values
                .filter { task in
                    task.deletedAt == nil && task.id != coded?.id
                        && words.allSatisfy { word in place(of: task).localizedStandardContains(word) }
                }
                .sorted { Self.order($0) < Self.order($1) }
            return ([coded].compactMap { $0 } + found).prefix(Self.limit).map { task in
                Match(id: task.id, code: workspace.displayCode(of: task), title: task.title, status: task.status)
            }
        }

        static let limit = 8

        /// Open tasks first, then by title.
        private static func order(_ task: LifrTask) -> (Int, String, String) {
            (task.isOpen ? 0 : 1, task.title, task.id.uuidString)
        }

        /// The words a task is found by.
        private func place(of task: LifrTask) -> String {
            let avenue = workspace.avenues[task.avenueID]?.name ?? ""
            let project = task.projectID.flatMap { workspace.projects[$0]?.name } ?? ""
            return [task.title, avenue, project].joined(separator: " ")
        }
    }

    package struct Match: Equatable, Identifiable, Sendable {
        package var id: UUID
        package var code: String
        package var title: String
        package var status: TaskStatus
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// Return in the field: the first match.
        case submitted
        case picked(UUID)
        case cancelTapped
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case open(UUID)
        }
    }

    @Dependency(\.dismiss) private var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding, .delegate:
                return .none
            case .submitted:
                guard let first = state.matches.first else { return .none }
                return .send(.picked(first.id))
            case .picked(let id):
                return .run { send in
                    await send(.delegate(.open(id)))
                    await dismiss()
                }
            case .cancelTapped:
                return .run { _ in await dismiss() }
            }
        }
    }
}

struct GoToTaskView: View {
    @Bindable var store: StoreOf<GoToTaskFeature>
    @FocusState private var isFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            TextField("Task code or title", text: $store.query)
                .textFieldStyle(.plain)
                .font(.title3)
                .focused($isFocused)
                .onSubmit { store.send(.submitted) }
                .padding(12)
            Divider()
            if store.query.trimmingCharacters(in: .whitespaces).isEmpty {
                hint("Type a code, such as 7K2Q, or words from a title")
            } else if store.matches.isEmpty {
                hint("No task matches")
            } else {
                List(store.matches) { match in
                    Button {
                        store.send(.picked(match.id))
                    } label: {
                        HStack {
                            StatusGlyph(match.status)
                            Text(match.title).lineLimit(1)
                            Spacer()
                            Text(match.code).font(.caption.monospaced()).foregroundStyle(.secondary)
                        }
                        .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                }
                .listStyle(.plain)
            }
        }
        #if os(macOS)
            .frame(minWidth: 420, minHeight: 280)
        #endif
        .onAppear { isFocused = true }
        #if os(macOS)
            .onExitCommand { store.send(.cancelTapped) }
        #endif
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { store.send(.cancelTapped) }
            }
        }
    }

    private func hint(_ text: LocalizedStringKey) -> some View {
        Text(text)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
