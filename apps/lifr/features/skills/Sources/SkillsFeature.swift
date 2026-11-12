import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrModelsClient

/// The assistant's skills: turn any on or off, and write your own.
@Reducer
package struct SkillsFeature {
    @ObservableState
    package struct State: Equatable {
        package var skills: [Skill] = []
        @Presents package var editor: SkillEditor.State?
        @Presents package var alert: AlertState<Never>?
        /// Why the skills couldn't be loaded, until they load.
        package var loadError: String?

        package init(skills: [Skill] = []) {
            self.skills = skills
        }
    }

    package enum Action {
        case task
        case loaded(Result<[Skill], any Error>)
        case toggled(String, Bool)
        /// Whether Lifr may pick the skill by itself was changed.
        case autoPicksToggled(String, Bool)
        case newTapped
        case editTapped(String)
        case deleteTapped([String])
        case editor(PresentationAction<SkillEditor.Action>)
        case saved(Result<[Skill], any Error>)
        case alert(PresentationAction<Never>)
    }

    @Dependency(\.skills) private var client

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                return .run { [client] send in await send(.loaded(Result { try await client.load() })) }

            case .loaded(.success(let skills)), .saved(.success(let skills)):
                state.skills = skills
                state.loadError = nil
                return .none

            case .toggled(let id, let on):
                guard let index = state.skills.firstIndex(where: { $0.id == id }) else { return .none }
                state.skills[index].isOn = on
                let skill = state.skills[index]
                return save { try await $0.save(skill) }

            case .autoPicksToggled(let id, let on):
                guard let index = state.skills.firstIndex(where: { $0.id == id }) else { return .none }
                state.skills[index].autoPicks = on
                let skill = state.skills[index]
                return save { try await $0.save(skill) }

            case .newTapped:
                state.editor = SkillEditor.State(taken: Set(state.skills.map(\.id)))
                return .none

            case .editTapped(let id):
                guard let skill = state.skills.first(where: { $0.id == id }), skill.isUser else { return .none }
                state.editor = SkillEditor.State(editing: skill)
                return .none

            case .deleteTapped(let ids):
                let ids = ids.filter { $0.hasPrefix(Skill.userPrefix) }
                state.skills.removeAll { ids.contains($0.id) }
                return save { try await $0.delete(ids) }

            case .editor(.presented(.delegate(.save(let skill)))):
                state.editor = nil
                return save { try await $0.save(skill) }

            case .loaded(.failure(let error)):
                state.loadError = String(describing: error)
                return .none

            case .saved(.failure(let error)):
                state.alert = AlertState {
                    TextState("Couldn't save the skill")
                } message: {
                    TextState(String(describing: error))
                }
                return .send(.task)

            case .editor, .alert:
                return .none
            }
        }
        .ifLet(\.$editor, action: \.editor) { SkillEditor() }
        .ifLet(\.$alert, action: \.alert)
    }

    /// Runs `change`, then shows the skills as stored.
    private func save(_ change: @escaping @Sendable (SkillsClient) async throws -> Void) -> Effect<Action> {
        .run { [client] send in
            await send(
                .saved(
                    Result {
                        try await change(client)
                        return try await client.load()
                    }))
        }
    }
}

/// Writes a skill of the person's, or changes one.
@Reducer
package struct SkillEditor {
    @ObservableState
    package struct State: Equatable {
        /// The skill being changed; nil for a new one.
        package var editing: Skill?
        /// IDs a new skill can't have.
        package var taken: Set<String> = []
        package var name = ""
        package var summary = ""
        package var instructions = ""
        package var tools: Set<AssistantTool> = [.findTasks, .getTask, .overview]
        package var draftsPlans = false
        package var autoPicks = false
        package var useWhen = ""
        /// Whether the person asked for the assistant's help, which is off
        /// until they do.
        package var assists = false
        /// What the person wants the skill to do, for the assistant.
        package var request = ""
        package var isDrafting = false
        /// Why the assistant couldn't write the skill.
        package var draftProblem: String?
        /// The model that drafts it this time, or nil for the one picked
        /// for skills in Settings.
        package var model: ModelID?
        /// Apple's model and the downloaded ones, once they're looked up.
        package var models: [RunnableModel] = []
        /// The model picked for skills in Settings.
        package var standard: ModelID?

        package init(taken: Set<String>) {
            self.taken = taken
        }

        package init(editing skill: Skill) {
            editing = skill
            name = skill.id
            summary = skill.summary
            instructions = skill.instructions
            tools = skill.tools
            draftsPlans = skill.draftsPlans
            autoPicks = skill.autoPicks
            useWhen = skill.useWhen
        }

        /// The skill as written, or nil when it can't be saved yet.
        package var skill: Skill? {
            let instructions = instructions.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !instructions.isEmpty else { return nil }
            let summary = summary.trimmingCharacters(in: .whitespacesAndNewlines)
            let useWhen = useWhen.trimmingCharacters(in: .whitespacesAndNewlines)
            if var skill = editing {
                skill.summary = summary
                skill.instructions = instructions
                skill.tools = tools
                skill.draftsPlans = draftsPlans
                skill.autoPicks = autoPicks
                skill.useWhen = useWhen
                return skill
            }
            return Skill(
                userNamed: name, summary: summary, instructions: instructions, tools: tools,
                draftsPlans: draftsPlans, autoPicks: autoPicks, useWhen: useWhen)
        }

        /// Why the skill can't be saved, once there's something to say.
        package var problem: String? {
            guard editing == nil, !name.isEmpty else { return nil }
            guard let id = Skill(userNamed: name, summary: "", instructions: "", tools: [], draftsPlans: false)?.id
            else { return String(localized: "Give the skill a name with letters or digits.") }
            return taken.contains(id)
                ? String(localized: "You already have a skill called \(id). Pick another name.")
                : nil
        }

        package var canSave: Bool { skill != nil && problem == nil && !isDrafting }

        package var canDraft: Bool {
            !isDrafting && !request.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case toolToggled(AssistantTool, Bool)
        case assistTapped
        case modelsLoaded([RunnableModel], standard: ModelID)
        case draftTapped
        case drafted(Result<SkillDraft, any Error>)
        case saveTapped
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case save(Skill)
        }
    }

    @Dependency(\.assistant) private var assistant
    @Dependency(\.models) private var models

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .assistTapped:
                state.assists = true
                checkAvailability(&state)
                return .run { [models] send in
                    await send(.modelsLoaded(await models.runnable(), standard: models.selection(.skills)))
                }

            case .modelsLoaded(let runnable, let standard):
                state.models = runnable
                state.standard = standard
                return .none

            case .binding(\.model):
                checkAvailability(&state)
                return .none

            case .draftTapped:
                guard state.canDraft else { return .none }
                state.isDrafting = true
                state.draftProblem = nil
                let request = state.request.trimmingCharacters(in: .whitespacesAndNewlines)
                let model = state.model
                return .run { [assistant] send in
                    await send(.drafted(Result { try await assistant.draftSkill(request, model) }))
                }

            case .drafted(.success(let draft)):
                state.isDrafting = false
                // An existing skill keeps its name, which is its ID.
                if state.editing == nil { state.name = draft.name }
                state.summary = draft.summary
                state.instructions = draft.instructions
                state.tools = draft.tools
                state.draftsPlans = draft.draftsPlans
                return .none

            case .drafted(.failure(let error)):
                state.isDrafting = false
                state.draftProblem = String(describing: error)
                return .none

            case .toolToggled(let tool, let on):
                if on { state.tools.insert(tool) } else { state.tools.remove(tool) }
                return .none

            case .saveTapped:
                guard state.canSave, let skill = state.skill else { return .none }
                return .send(.delegate(.save(skill)))

            case .binding, .delegate:
                return .none
            }
        }
    }

    /// Says why the model that drafts can't run now, if it can't.
    private func checkAvailability(_ state: inout State) {
        let availability = assistant.availability(ModelChoice(model: state.model ?? state.standard))
        state.draftProblem = availability == .available ? nil : availability.description
    }
}

extension AssistantTool {
    /// What the tool lets the assistant do, as Settings lists it.
    package var title: String {
        switch self {
        case .findTasks: String(localized: "Search tasks")
        case .getTask: String(localized: "Read a task")
        case .overview: String(localized: "See what's due")
        case .recentActivity: String(localized: "See recent changes")
        case .updateTasks: String(localized: "Change tasks")
        case .updateProjects: String(localized: "Change Projects")
        case .addComment: String(localized: "Comment on tasks")
        case .revert: String(localized: "Undo changes")
        }
    }
}
