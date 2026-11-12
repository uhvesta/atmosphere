import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// A project's name, its instructions and its skills: what every session
/// in it reads, from the next turn each starts.
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
        }

        package var canSave: Bool {
            !saving && !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case addSkillTapped
        case removeSkillTapped(SkillRow.ID)
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

            case .saveTapped:
                guard state.canSave else { return .none }
                state.saving = true
                let skills = state.skills.compactMap { row -> Skill? in
                    let name = row.name.trimmingCharacters(in: .whitespacesAndNewlines)
                    let text = row.text.trimmingCharacters(in: .whitespacesAndNewlines)
                    return name.isEmpty || text.isEmpty ? nil : Skill(name: name, text: text)
                }
                let project = state.project
                let name = state.name.trimmingCharacters(in: .whitespacesAndNewlines)
                let update = ProjectUpdate(
                    name: name == project.name ? nil : name,
                    instructions: state.instructions == project.instructions ? nil : state.instructions,
                    skills: skills == project.skills ? nil : skills)
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

/// The sheet to change a project's name, instructions and skills.
struct ProjectSettingsView: View {
    @Bindable var store: StoreOf<ProjectSettingsFeature>
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Form {
            Section {
                TextField("Name", text: $store.name)
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
