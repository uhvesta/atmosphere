import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// A project's name, its repositories, its instructions and its skills:
/// what every session in it reads, from the next turn each starts, and
/// what sessions started after work in.
@Reducer
package struct ProjectSettingsFeature {
    /// A skill as it's being written; one left empty is dropped on saving.
    package struct SkillRow: Identifiable, Hashable, Sendable {
        package var id: Int
        package var name: String
        package var text: String

        package init(id: Int, name: String = "", text: String = "") {
            self.id = id
            self.name = name
            self.text = text
        }
    }

    @ObservableState
    package struct State: Equatable {
        package var project: Project
        package var name: String
        package var instructions: String
        package var skills: IdentifiedArrayOf<SkillRow>
        var nextRow: Int
        /// Each a folder on this Mac or the URL it was added by.
        package var repositories: IdentifiedArrayOf<RepositoryRow>
        var nextRepository: Int
        package var saving = false
        @Presents package var alert: AlertState<Never>?

        package init(project: Project) {
            self.project = project
            name = project.name
            instructions = project.instructions
            skills = IdentifiedArray(
                uniqueElements: project.skills.enumerated().map { index, skill in
                    SkillRow(id: index, name: skill.name, text: skill.text)
                })
            nextRow = project.skills.count
            repositories = Self.rows(project)
            nextRepository = project.repositories.count
        }

        static func rows(_ project: Project) -> IdentifiedArrayOf<RepositoryRow> {
            IdentifiedArray(
                uniqueElements: project.repositories.enumerated().map { index, repository in
                    RepositoryRow(
                        id: index, name: repository.name,
                        path: repository.url.isEmpty ? repository.path : repository.url)
                })
        }

        package var canSave: Bool {
            !saving && !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && repositories.contains { $0.repository != nil }
                && repositories.allSatisfy { $0.path.trimmed.isEmpty || $0.repository != nil }
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case addSkillTapped
        case removeSkillTapped(SkillRow.ID)
        case addRepositoryTapped
        case removeRepositoryTapped(RepositoryRow.ID)
        case saveTapped
        case failed(String)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case saved(Project)
        }
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding, .alert, .delegate:
                return .none

            case .addSkillTapped:
                state.skills.append(SkillRow(id: state.nextRow))
                state.nextRow += 1
                return .none

            case .removeSkillTapped(let id):
                state.skills.remove(id: id)
                return .none

            case .addRepositoryTapped:
                state.repositories.append(RepositoryRow(id: state.nextRepository))
                state.nextRepository += 1
                return .none

            case .removeRepositoryTapped(let id):
                state.repositories.remove(id: id)
                return .none

            case .saveTapped:
                guard state.canSave else { return .none }
                state.saving = true
                let skills = state.skills.compactMap { row -> Skill? in
                    let name = row.name.trimmingCharacters(in: .whitespacesAndNewlines)
                    let text = row.text.trimmingCharacters(in: .whitespacesAndNewlines)
                    return name.isEmpty || text.isEmpty ? nil : Skill(name: name, text: text)
                }
                let project = state.project
                let repositories = state.repositories.compactMap(\.repository)
                let unchanged = State.rows(project).compactMap(\.repository)
                let name = state.name.trimmingCharacters(in: .whitespacesAndNewlines)
                let update = ProjectUpdate(
                    name: name == project.name ? nil : name,
                    instructions: state.instructions == project.instructions ? nil : state.instructions,
                    skills: skills == project.skills ? nil : skills,
                    repositories: repositories == unchanged ? nil : repositories)
                return .run { send in
                    await send(.delegate(.saved(try await daemon.updateProject(project.id, update))))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .failed(let message):
                state.saving = false
                state.alert = AlertState {
                    TextState("Couldn't save the project")
                } message: {
                    TextState(message)
                }
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }
}

/// The sheet to change a project's name, repositories, instructions and
/// skills.
struct ProjectSettingsView: View {
    @Bindable var store: StoreOf<ProjectSettingsFeature>
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Form {
            Section {
                TextField("Name", text: $store.name)
            }
            Section {
                ForEach($store.repositories) { $row in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            TextField("Folder", text: $row.name, prompt: Text("Named after it"))
                                .labelsHidden()
                                .frame(maxWidth: 120)
                            TextField(
                                "Repository", text: $row.path,
                                prompt: Text("/path/to/repository or https://github.com/owner/name")
                            )
                            .labelsHidden()
                            Button {
                                store.send(.removeRepositoryTapped(row.id))
                            } label: {
                                Label("Remove", systemImage: "minus.circle")
                                    .labelStyle(.iconOnly)
                            }
                            .buttonStyle(.borderless)
                            .disabled(store.repositories.count == 1)
                            .help("Leave this repository out of sessions started after")
                        }
                        if row.needsFullPath { FullPathNote() }
                    }
                }
                Button("Add Repository") { store.send(.addRepositoryTapped) }
            } header: {
                Text("Repositories")
            } footer: {
                Text(
                    "Each a folder on this Mac or a URL to clone, and the name of its folder in a session's copy. "
                        + "Sessions started after saving work in these; ones already started keep their copies."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            Section {
                TextEditor(text: $store.instructions)
                    .frame(minHeight: 80)
                    .accessibilityLabel("Instructions")
            } header: {
                Text("Instructions")
            } footer: {
                Text("Every session in the project reads these first, then the skills, then its task's spec.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Section {
                ForEach($store.skills) { $skill in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            TextField("Skill", text: $skill.name, prompt: Text("Running the tests"))
                                .labelsHidden()
                            Button {
                                store.send(.removeSkillTapped(skill.id))
                            } label: {
                                Label("Remove", systemImage: "minus.circle")
                                    .labelStyle(.iconOnly)
                            }
                            .buttonStyle(.borderless)
                            .help("Remove this skill from the project")
                        }
                        TextEditor(text: $skill.text)
                            .font(.callout.monospaced())
                            .frame(minHeight: 60)
                            .accessibilityLabel("\(skill.name.isEmpty ? "Skill" : skill.name) text")
                    }
                }
                Button("Add Skill") { store.send(.addSkillTapped) }
            } header: {
                Text("Skills")
            } footer: {
                Text("Know-how for the agents, such as how to run the tests or deploy. A skill left empty is dropped.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .frame(minWidth: 480, minHeight: 420)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Save") { store.send(.saveTapped) }
                    .disabled(!store.canSave)
            }
        }
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}
