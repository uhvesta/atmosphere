import ComposableArchitecture
import Foundation
import LifrAPI
import LifrAssistantClient
import LifrCore
import LifrPasteboardClient
import LifrSkills

/// A prompt for planning with an assistant outside Lifr: the person picks
/// which tasks it lists and sees it before copying or sharing it.
@Reducer
package struct PlanExportFeature {
    @ObservableState
    package struct State: Equatable {
        package var goal: String
        package var listed: PlanPrompt.Scope = .names
        /// Leaves closed tasks out of the listed ones.
        package var openOnly = true
        /// Adds each listed task's description.
        package var descriptions = false
        /// The skill whose instructions the prompt carries.
        package var preset: Skill?
        package var workspace: Workspace
        var today: CalendarDate
        var timeZone: TimeZone
        /// The prompt as the choices above make it.
        package private(set) var prompt = ""
        /// Whether the prompt was copied since it last changed.
        package var copied = false
        @Presents package var alert: AlertState<Never>?
        /// Whether `preset` keeps the goal and scope as they are now.
        package var remembered = false

        /// With a `preset`, an empty `goal` starts as the preset's, and the
        /// listed tasks as the preset's scope while it still names an
        /// Avenue or Project the prompt can list.
        package init(goal: String, preset: Skill?, workspace: Workspace, today: CalendarDate, timeZone: TimeZone) {
            let trimmed = goal.trimmingCharacters(in: .whitespacesAndNewlines)
            self.goal = trimmed.isEmpty ? preset?.planGoal ?? goal : goal
            self.preset = preset
            self.workspace = workspace
            self.today = today
            self.timeZone = timeZone
            if let stored = preset?.planScope, let scope = PlanPrompt.Scope(stored: stored), canList(scope) {
                listed = scope
            }
            remake()
            remembered = preset.map(keeps) ?? false
        }

        /// Whether the prompt can list `scope`: an Avenue or Project that's
        /// gone or closed to the assistant can't be.
        func canList(_ scope: PlanPrompt.Scope) -> Bool {
            switch scope {
            case .names, .everything: true
            case .avenue(let id): avenues.contains { $0.id == id }
            case .project(let id): projects.contains { $0.project.id == id }
            }
        }

        /// Whether `skill` keeps this goal and scope.
        func keeps(_ skill: Skill) -> Bool {
            let goal = goal.trimmingCharacters(in: .whitespacesAndNewlines)
            return (skill.planGoal ?? "") == goal && (skill.planScope ?? PlanPrompt.Scope.names.stored) == listed.stored
        }

        /// The Avenues the prompt can list, which are the ones an
        /// assistant may add to.
        package var avenues: [Avenue] {
            workspace.avenues.values
                .filter { $0.archivedAt == nil && $0.agentAccess == .full }
                .sorted { $0.sortOrder < $1.sortOrder }
        }

        /// The active Projects in `avenues`, by Avenue then name.
        package var projects: [(avenue: Avenue, project: Project)] {
            avenues.flatMap { avenue in
                workspace.projects.values
                    .filter { $0.avenueID == avenue.id && $0.status == .active }
                    .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
                    .map { (avenue, $0) }
            }
        }

        /// Whether the prompt lists any tasks.
        package var listsTasks: Bool { listed != .names }

        mutating func remake() {
            prompt = PlanPrompt.make(
                goal: goal, scope: listed, listing: PlanPrompt.Listing(closed: !openOnly, descriptions: descriptions),
                instructions: preset?.instructions ?? "", workspace: workspace, today: today, timeZone: timeZone)
            copied = false
            remembered = preset.map(keeps) ?? false
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case copyTapped
        case doneTapped
        /// Keeps the goal and scope in the preset, to start from next time.
        case rememberTapped
        case remembered(Result<Skill, any Error>)
        case delegate(Delegate)
        case alert(PresentationAction<Never>)

        package enum Delegate: Equatable {
            /// The preset now keeps a goal and scope.
            case presetSaved(Skill)
        }
    }

    @Dependency(\.pasteboard) private var pasteboard
    @Dependency(\.skills) private var skills
    @Dependency(\.dismiss) private var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding(\.copied):
                return .none

            case .binding:
                state.remake()
                return .none

            case .copyTapped:
                state.copied = true
                let prompt = state.prompt
                return .run { [pasteboard] _ in await pasteboard.copy(prompt) }

            case .doneTapped:
                return .run { [dismiss] _ in await dismiss() }

            case .rememberTapped:
                guard var preset = state.preset, preset.isUser else { return .none }
                let goal = state.goal.trimmingCharacters(in: .whitespacesAndNewlines)
                preset.planGoal = goal.isEmpty ? nil : goal
                preset.planScope = state.listed == .names ? nil : state.listed.stored
                return .run { [skills, preset] send in
                    await send(.remembered(Result { try await skills.save(preset) }.map { preset }))
                }

            case .remembered(.success(let preset)):
                state.preset = preset
                state.remembered = state.keeps(preset)
                return .send(.delegate(.presetSaved(preset)))

            case .remembered(.failure(let error)):
                state.alert = AlertState {
                    TextState("Couldn't keep the goal and scope")
                } actions: {
                    ButtonState(role: .cancel) { TextState("OK") }
                } message: {
                    TextState(
                        "\(String(describing: error)) The skill is as it was; the prompt here still works. Try again, "
                            + "or set them each time.")
                }
                return .none

            case .delegate, .alert:
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }
}
