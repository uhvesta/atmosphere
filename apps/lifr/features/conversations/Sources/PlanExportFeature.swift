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

        package init(goal: String, preset: Skill?, workspace: Workspace, today: CalendarDate, timeZone: TimeZone) {
            self.goal = goal
            self.preset = preset
            self.workspace = workspace
            self.today = today
            self.timeZone = timeZone
            remake()
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
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case copyTapped
        case doneTapped
    }

    @Dependency(\.pasteboard) private var pasteboard
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
            }
        }
    }
}
