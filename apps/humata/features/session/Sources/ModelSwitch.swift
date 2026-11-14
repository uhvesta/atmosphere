import ChatUI
import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// Changing an open chat's model from the composer's pill: the session's
/// backend's models, listed by its host, and the change, which the daemon
/// keeps and makes from the next turn.
package struct ModelSwitch: Equatable, Sendable {
    /// The list of models is open.
    package var isPicking = false
    /// What the session can change to, as its host last listed them; nil
    /// until it has.
    package var models: [ModelChoice]?
    /// The host is being asked for them.
    package var loading = false
    /// The daemon is being asked to change the model.
    package var changing = false
    /// Why the models couldn't be listed or the model wasn't changed.
    package var failure: String?

    package init() {}

    /// Whether `session`'s model can be changed: its backend picks among
    /// models, and its host is there to list them and take the change.
    package static func offered(for session: Session) -> Bool {
        guard !session.archived, !session.hostOffline else { return false }
        switch session.backend {
        case .claude, .copilot, .local: return true
        case .scripted: return false
        }
    }

    /// Whether the backend runs with no model named, on its own default.
    /// A model server has none.
    package static func offersDefault(_ backend: Backend) -> Bool {
        switch backend {
        case .claude, .copilot: true
        case .local, .scripted: false
        }
    }

    /// What the pill calls the session's model.
    func name(of session: Session) -> String {
        guard let id = session.model else { return "Default" }
        return models?.first { $0.id == id }?.name ?? id
    }
}

@CasePathable
package enum ModelSwitchAction: Sendable {
    case pillTapped
    case listed([ModelChoice])
    case listFailed(String)
    case tryAgainTapped
    /// A model's id, or nil for the backend's default.
    case picked(String?)
    /// The daemon answered with the session as it is now.
    case changed(Session, asked: String?)
    case changeFailed(String)
    case closed
}

extension SessionFeature {
    func modelSwitch(_ state: inout State, _ action: ModelSwitchAction) -> Effect<Action> {
        switch action {
        case .pillTapped:
            guard ModelSwitch.offered(for: state.session) else { return .none }
            state.modelSwitch.isPicking = true
            state.modelSwitch.failure = nil
            return listModels(&state)

        case .tryAgainTapped:
            state.modelSwitch.failure = nil
            return listModels(&state)

        case .listed(let models):
            state.modelSwitch.loading = false
            state.modelSwitch.models = models
            return .none

        case .listFailed(let message):
            state.modelSwitch.loading = false
            state.modelSwitch.failure = message
            return .none

        case .picked(let id):
            guard !state.modelSwitch.changing, ModelSwitch.offered(for: state.session) else { return .none }
            guard id != state.session.model else {
                state.modelSwitch.isPicking = false
                return .none
            }
            state.modelSwitch.changing = true
            state.modelSwitch.failure = nil
            let session = state.session
            // Another model's effort may not be one this one takes; one
            // it isn't known to take goes back to the model's default.
            let efforts = state.modelSwitch.models?.first { $0.id == id }?.efforts ?? []
            let dropsEffort = session.effort.map { !efforts.contains($0) } ?? false
            // Sent at once, also while a turn runs: the daemon keeps that
            // turn on its model and starts the next, queued ones too, on
            // this one.
            return .run { send in
                let update = SessionUpdate(effort: dropsEffort ? "" : nil, model: id ?? "")
                let updated = try await daemon.update(session.id, update)
                await send(.model(.changed(updated, asked: id)))
            } catch: { error, send in
                await send(.model(.changeFailed(error.localizedDescription)))
            }

        case .changed(let updated, let asked):
            state.modelSwitch.changing = false
            guard updated.model == asked else {
                // A humatad from before models could be changed takes the
                // request and leaves the model as it was.
                state.modelSwitch.failure =
                    state.session.host.map {
                        "The model wasn’t changed: humatad on \($0) is older than this app. Install Humata on the "
                            + "host again from Settings › Environments, then pick the model again."
                    }
                    ?? "The model wasn’t changed: the Humata daemon that is running is older than this app. Quit "
                    + "and reopen Humata once no turn is running, then pick the model again."
                return .none
            }
            if state.session.model != updated.model { state.session.model = updated.model }
            if state.session.effort != updated.effort { state.session.effort = updated.effort }
            state.modelSwitch.isPicking = false
            // Whether images can be sent goes by the model.
            switch state.session.backend {
            case .claude, .scripted:
                return .none
            case .copilot, .local:
                if let choice = state.modelSwitch.models?.first(where: { $0.id == asked }) {
                    return .send(.readsImagesFound(choice.readsImages))
                }
                return .send(.readsImagesAsked)
            }

        case .changeFailed(let message):
            state.modelSwitch.changing = false
            state.modelSwitch.failure = message
            return .none

        case .closed:
            state.modelSwitch.isPicking = false
            return .none
        }
    }

    /// Asked each time the list opens, behind what was last listed: the
    /// models a host offers come and go.
    private func listModels(_ state: inout State) -> Effect<Action> {
        guard !state.modelSwitch.loading else { return .none }
        state.modelSwitch.loading = true
        let session = state.session
        return .run { send in
            let models = try await daemon.sessionModels(session)
            await send(.model(.listed(models)))
        } catch: { error, send in
            await send(.model(.listFailed(error.localizedDescription)))
        }
    }
}

extension ModelSwitch {
    /// The composer's pill for the session's model, where it can be
    /// changed; nil where it can't, so nothing is offered that won't work.
    @MainActor
    static func pill(_ store: StoreOf<SessionFeature>) -> ModelPill? {
        let session = store.session
        guard offered(for: session) else { return nil }
        let effort = session.effort.map { $0 == "xhigh" ? "Extra High" : $0.capitalized }
        return ModelPill(name: store.modelSwitch.name(of: session), detail: effort) {
            store.send(.model(.pillTapped))
        }
    }
}

/// Shows the list of models over the session while one is being picked.
struct ModelSwitchSheet: ViewModifier {
    let store: StoreOf<SessionFeature>

    func body(content: Content) -> some View {
        content.sheet(
            isPresented: Binding(
                get: { store.modelSwitch.isPicking },
                set: { shown in
                    if !shown { store.send(.model(.closed)) }
                })
        ) {
            ModelSwitchList(store: store)
        }
    }
}

/// The models the session can change to, with a check on the one it has;
/// picking one changes it and closes the list.
private struct ModelSwitchList: View {
    let store: StoreOf<SessionFeature>

    var body: some View {
        let picking = store.modelSwitch
        let session = store.session
        VStack(alignment: .leading, spacing: 12) {
            Text("Select Model")
                .font(.headline)
            Text(
                store.isWorking
                    ? "The turn that is running finishes on the model it started with. Messages after it, queued "
                        + "ones too, use the model you pick."
                    : "The next message uses the model you pick."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
            if let models = picking.models {
                let offersDefault = ModelSwitch.offersDefault(session.backend)
                if models.isEmpty, !offersDefault {
                    Text("The session’s host lists no models to change to.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                } else {
                    ScrollView {
                        VStack(spacing: 2) {
                            if offersDefault {
                                row(id: nil, name: "Default", detail: "The agent’s own choice of model")
                            }
                            ForEach(models) { model in
                                row(id: model.id, name: model.name, detail: model.detail)
                            }
                        }
                    }
                    .frame(maxHeight: 340)
                    .disabled(picking.changing)
                }
            } else if picking.loading {
                Label {
                    Text("Listing the models…")
                } icon: {
                    ProgressView().controlSize(.small)
                }
                .font(.callout)
                .foregroundStyle(.secondary)
            }
            if let failure = picking.failure {
                Label(failure, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                if picking.changing {
                    ProgressView().controlSize(.small)
                    Text("Changing the model…")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                } else if picking.failure != nil, picking.models == nil {
                    Button("Try Again") { store.send(.model(.tryAgainTapped)) }
                }
                Spacer()
                Button("Cancel") { store.send(.model(.closed)) }
                    .keyboardShortcut(.cancelAction)
            }
        }
        .padding(20)
        .frame(width: 440)
        .accessibilityIdentifier("composer.models")
    }

    private func row(id: String?, name: String, detail: String?) -> some View {
        let picked = store.session.model == id
        return Button {
            store.send(.model(.picked(id)))
        } label: {
            HStack(spacing: 8) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(name)
                    if let detail {
                        Text(detail)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                Spacer(minLength: 8)
                if picked {
                    Image(systemName: "checkmark")
                        .foregroundStyle(.tint)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(picked ? AnyShapeStyle(.fill.tertiary) : AnyShapeStyle(.clear), in: .rect(cornerRadius: 6))
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(picked ? .isSelected : [])
    }
}
