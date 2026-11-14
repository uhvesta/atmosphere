import ComposableArchitecture
import Foundation
import HumataDaemonClient
import SwiftUI

/// How hard a model thinks, from those it takes; for Copilot Fusion, its
/// tier, whose cost is that of the models it picks.
struct EffortPicker: View {
    var model: ModelChoice
    var fusion: Bool
    @Binding var effort: String

    var body: some View {
        Picker(fusion ? "Tier" : "Effort", selection: $effort) {
            Text(model.defaultEffort.map { "Default (\(Self.title($0)))" } ?? "Default").tag("")
            ForEach(model.efforts, id: \.self) { effort in
                Text(Self.title(effort)).tag(effort)
            }
        }
        .help(
            fusion
                ? "How many models Fusion weighs for each turn. Higher tiers use more of them, and each is billed "
                    + "as it would be on its own."
                : "How long the model thinks before it answers. More effort takes longer and costs more.")
    }

    /// `xhigh` as "Extra High", and the rest capitalized.
    static func title(_ effort: String) -> String {
        effort == "xhigh" ? "Extra High" : effort.capitalized
    }
}

/// Settings › Models: the agent, model and effort New Session starts from,
/// in every project or, over those, in one.
@Reducer
package struct ModelDefaultsFeature {
    @ObservableState
    package struct State: Equatable {
        package var loading = true
        package var projects: [Project] = []
        /// The project whose defaults are shown; nil for every project's.
        package var projectID: String?
        /// Every project's.
        package var every = SessionDefaults()
        /// What's shown: every project's, or the project's own.
        package var defaults = SessionDefaults()
        /// For a project: whether it has its own, or goes by every project's.
        package var own = false
        /// The model typed or picked, as the field shows it.
        package var model = ""
        /// What the agent offers on this Mac, to pick the model from.
        package var models: [ModelChoice] = []
        /// Whether that list is being read.
        package var modelsLoading = false
        /// Why they couldn't be read or saved, and what to do.
        package var failure: String?
        /// Counts the changes made here, so a load or a pause in typing
        /// that began before the latest one knows it's out of date.
        var edits = 0
        /// The change waiting for typing to settle.
        var typed: Write?
        /// Changes waiting for the one being written. Writes go one at a
        /// time, so an older one can't land after a newer one and undo it.
        var queued: [Write] = []
        var writing = false

        package init() {}

        package var saving: Bool { writing || typed != nil || !queued.isEmpty }
        package var project: Project? { projects.first { $0.id == projectID } }
        /// Whether the fields can change: always for every project, and
        /// for a project once it has its own.
        package var editable: Bool { projectID == nil || own }
        package var pickedModel: ModelChoice? { models.first { $0.id == defaults.model } }
        package var fusion: Bool { defaults.backend == .copilot && defaults.model == ModelChoice.fusion }
    }

    /// Defaults to store: every project's, when `project` is nil, or one
    /// project's own, with nil defaults for it to go by every project's.
    struct Write: Equatable {
        var project: String?
        var defaults: SessionDefaults?
    }

    package enum Action {
        case task
        case loaded(edits: Int, SessionDefaults, [Project])
        case failed(String)
        case projectPicked(String?)
        case ownToggled(Bool)
        case backendPicked(Backend?)
        case modelTyped(String)
        case typingSettled(edits: Int)
        case effortPicked(String)
        case experimentalToggled(Bool)
        case modelsLoaded(Backend, [ModelChoice])
        case saved(Project?)
        case saveFailed(String)
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.continuousClock) var clock

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                state.loading = true
                // A list asked for when the tab last showed stopped with it.
                state.modelsLoading = false
                let edits = state.edits
                return .run { send in
                    async let projects = daemon.projects()
                    let every = try await daemon.sessionDefaults()
                    await send(.loaded(edits: edits, every, try await projects))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .loaded(let edits, let every, let projects):
                state.loading = false
                state.projects = projects.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
                if state.projectID != nil, state.project == nil { state.projectID = nil }
                // Read before a change made since, as when the tab shows
                // again mid-edit: showing it would put the old ones back.
                guard edits == state.edits else { return .none }
                state.every = every
                return show(&state)

            case .failed(let message):
                state.loading = false
                state.failure = message
                return .none

            case .projectPicked(let id):
                // What was typed for the last one is saved before leaving it.
                let flush = flushTyped(&state)
                state.projectID = id
                return .merge(flush, show(&state))

            case .ownToggled(let own):
                state.own = own
                // Its own start from every project's, to change from there;
                // without, it shows every project's again.
                state.defaults = state.every
                state.model = state.defaults.model ?? ""
                return .merge(save(&state), loadModels(&state))

            case .backendPicked(let backend):
                state.defaults = SessionDefaults(backend: backend, experimental: state.defaults.experimental)
                state.model = ""
                return .merge(save(&state), loadModels(&state))

            case .modelTyped(let model):
                state.model = model
                let trimmed = model.trimmingCharacters(in: .whitespacesAndNewlines)
                state.defaults.model = trimmed.isEmpty ? nil : trimmed
                if let effort = state.defaults.effort, state.pickedModel?.efforts.contains(effort) == false {
                    state.defaults.effort = nil
                }
                // Typed a letter at a time; saves once it settles.
                return save(&state, after: .milliseconds(500))

            case .typingSettled(let edits):
                guard edits == state.edits else { return .none }
                return flushTyped(&state)

            case .effortPicked(let effort):
                state.defaults.effort = effort.isEmpty ? nil : effort
                return save(&state)

            case .experimentalToggled(let on):
                state.defaults.experimental = on
                return save(&state)

            case .modelsLoaded(let backend, let models):
                // A list for an agent picked before this one.
                guard backend == state.defaults.backend else { return .none }
                state.modelsLoading = false
                state.models = models
                return .none

            case .saved(let project):
                state.writing = false
                if let project, let index = state.projects.firstIndex(where: { $0.id == project.id }) {
                    state.projects[index] = project
                }
                return writeNext(&state)

            case .saveFailed(let message):
                state.writing = false
                state.failure = message
                return writeNext(&state)
            }
        }
    }

    /// Shows the defaults of the project picked, or every project's.
    private func show(_ state: inout State) -> Effect<Action> {
        if let project = state.project {
            state.own = project.defaults != nil
            state.defaults = project.defaults ?? state.every
        } else {
            state.own = false
            state.defaults = state.every
        }
        state.model = state.defaults.model ?? ""
        return loadModels(&state)
    }

    private func save(_ state: inout State, after delay: Duration? = nil) -> Effect<Action> {
        let project = state.projectID
        let write = Write(project: project, defaults: project == nil || state.own ? state.defaults : nil)
        if project == nil { state.every = state.defaults }
        state.edits += 1
        state.failure = nil
        state.loading = false
        // This change carries whatever was typed, so that needn't go too.
        state.typed = nil
        guard let delay else { return enqueue(&state, write) }
        state.typed = write
        let edits = state.edits
        return .run { send in
            try await clock.sleep(for: delay)
            await send(.typingSettled(edits: edits))
        }
    }

    private func flushTyped(_ state: inout State) -> Effect<Action> {
        guard let typed = state.typed else { return .none }
        state.typed = nil
        return enqueue(&state, typed)
    }

    private func enqueue(_ state: inout State, _ write: Write) -> Effect<Action> {
        state.queued.removeAll { $0.project == write.project }
        state.queued.append(write)
        return writeNext(&state)
    }

    private func writeNext(_ state: inout State) -> Effect<Action> {
        guard !state.writing, !state.queued.isEmpty else { return .none }
        state.writing = true
        let write = state.queued.removeFirst()
        return .run { send in
            await send(.saved(try await daemon.setSessionDefaults(write.project, write.defaults)))
        } catch: { error, send in
            await send(.saveFailed(error.localizedDescription))
        }
    }

    private func loadModels(_ state: inout State) -> Effect<Action> {
        guard let backend = state.defaults.backend, backend != .scripted else {
            state.models = []
            state.modelsLoading = false
            return .none
        }
        state.modelsLoading = true
        return .run { send in
            let models = (try? await daemon.models(nil, backend, nil)) ?? []
            await send(.modelsLoaded(backend, models))
        }
    }
}

package struct ModelDefaultsView: View {
    let store: StoreOf<ModelDefaultsFeature>

    package init(store: StoreOf<ModelDefaultsFeature>) {
        self.store = store
    }

    package var body: some View {
        Group {
            if store.loading {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                form
            }
        }
        .task { await store.send(.task).finish() }
    }

    private var form: some View {
        Form {
            if let failure = store.failure {
                Section {
                    Label(failure, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                }
            }
            Section {
                Picker(
                    "For",
                    selection: Binding(get: { store.projectID }, set: { store.send(.projectPicked($0)) })
                ) {
                    Text("Every Project").tag(String?.none)
                    ForEach(store.projects) { project in
                        Text(project.name).tag(String?.some(project.id))
                    }
                }
                if let project = store.project {
                    Toggle(
                        "Its Own Defaults",
                        isOn: Binding(get: { store.own }, set: { store.send(.ownToggled($0)) })
                    )
                    .help("Off, sessions in \(project.name) start from every project's.")
                }
            } footer: {
                Text(
                    "New Session starts from these. A project's go over every project's, and you can still pick "
                        + "another agent, model or effort for each session."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            Section {
                fields
                    .disabled(!store.editable)
            } header: {
                HStack {
                    Text("Starts With")
                    if store.saving { ProgressView().controlSize(.small) }
                }
            }
        }
        .formStyle(.grouped)
    }

    @ViewBuilder private var fields: some View {
        Picker(
            "Agent",
            selection: Binding(get: { store.defaults.backend }, set: { store.send(.backendPicked($0)) })
        ) {
            Text("None").tag(Backend?.none)
            Text("Claude Code").tag(Backend?.some(.claude))
            Text("Copilot").tag(Backend?.some(.copilot))
            Text("Local Model").tag(Backend?.some(.local))
        }
        if store.defaults.backend != nil {
            LabeledContent("Model") {
                HStack {
                    TextField(
                        "Model", text: Binding(get: { store.model }, set: { store.send(.modelTyped($0)) }),
                        prompt: Text("The agent's default")
                    )
                    .labelsHidden()
                    if !store.models.isEmpty {
                        ModelMenu(models: store.models, picked: store.model) { store.send(.modelTyped($0)) }
                    }
                }
            }
            if store.modelsLoading {
                LabeledContent("") {
                    HStack(spacing: 6) {
                        ProgressView().controlSize(.small)
                        Text("Listing models…").font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            if let detail = store.pickedModel?.detail {
                LabeledContent("") {
                    Text(detail).font(.caption).foregroundStyle(.secondary)
                }
            }
            if let picked = store.pickedModel, !picked.efforts.isEmpty {
                EffortPicker(
                    model: picked, fusion: store.fusion,
                    effort: Binding(
                        get: { store.defaults.effort ?? "" }, set: { store.send(.effortPicked($0)) }))
            }
        }
        if store.defaults.backend == .copilot {
            Toggle(
                isOn: Binding(get: { store.defaults.experimental }, set: { store.send(.experimentalToggled($0)) })
            ) {
                Text("Experimental Features")
                Text("Copilot's features still being tried out. Fusion needs them to pick local models.")
            }
        }
    }
}
