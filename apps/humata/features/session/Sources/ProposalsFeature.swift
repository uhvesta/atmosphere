import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// The tasks and spec edits a session's agent proposed, shown as cards in
/// its chat, or an outside agent proposed through `humata mcp`, shown on
/// the project's board: each applied as proposed or as edited here, or
/// declined. Nothing reaches the board until it is applied.
@Reducer
package struct ProposalsFeature {
    /// A proposal opened to change before applying it.
    package struct Editing: Equatable, Sendable {
        package var id: String
        package var title: String
        package var spec: String

        package init(id: String, title: String, spec: String) {
            self.id = id
            self.title = title
            self.spec = spec
        }
    }

    @ObservableState
    package struct State: Equatable {
        /// Empty for outside agents' proposals.
        package var sessionID: String
        /// The project whose outside agents' proposals these are; nil for a
        /// session's.
        package var projectID: String?
        package var proposals: IdentifiedArrayOf<Proposal> = []
        /// What each proposed spec edit takes out of the spec and puts in,
        /// worked out when the proposals are listed. A spec edit without
        /// one here hasn't had it worked out yet.
        package var changes: [Proposal.ID: ToolCall.Change] = [:]
        package var editing: Editing?
        /// The proposals being applied or declined, or having work started
        /// on their task, so their buttons wait.
        package var deciding: Set<String> = []
        @Presents package var alert: AlertState<Never>?
        /// Why they couldn't be listed, shown where they go; listing is
        /// tried again with each refresh.
        package var listFailure: String?

        package init(sessionID: String) {
            self.sessionID = sessionID
        }

        /// What outside agents proposed for `projectID`'s board.
        package init(outsideAgentsOf projectID: String) {
            self.sessionID = ""
            self.projectID = projectID
        }

        /// The proposals not decided yet, oldest first.
        package var pending: [Proposal] { proposals.filter { $0.state == .pending } }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// Lists them again: on opening, and when the agent proposes more.
        case refresh
        /// They were read, with what each spec edit among them changes.
        case listed([Proposal], changes: [Proposal.ID: ToolCall.Change])
        case applyTapped(String)
        case editTapped(String)
        case editCancelled
        /// Applies the proposal being edited, as edited.
        case editApplied
        case declineTapped(String)
        /// Forks the planning session onto the task applying it made.
        case startWorkTapped(itemID: String)
        /// The session on the task was started.
        case workStarted(proposal: String, Session)
        case decided(String, Proposal.State)
        /// They couldn't be listed.
        case listFailed(String)
        case failed(id: String, message: String)
        case startWorkFailed(proposal: String, message: String)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            /// A session was started on an applied task; open it.
            case openSession(Session)
        }
    }

    @Dependency(\.daemon) var daemon

    private enum CancelID { case list }

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding, .alert, .delegate:
                return .none

            case .refresh:
                let session = state.sessionID
                let project = state.projectID
                let known = state.proposals
                let kept = state.changes
                return .run { send in
                    let proposals: [Proposal]
                    if let project {
                        proposals = try await daemon.outsideProposals(project)
                    } else {
                        proposals = try await daemon.proposals(session)
                    }
                    // Off the main thread: a long spec's diff would stall
                    // the chat.
                    let changes = await Task.detached(priority: .userInitiated) {
                        Self.changes(of: proposals, known: known, kept: kept)
                    }.value
                    await send(.listed(proposals, changes: changes))
                } catch: { error, send in
                    await send(.listFailed(error.localizedDescription))
                }
                .cancellable(id: CancelID.list, cancelInFlight: true)

            case .listFailed(let message):
                if state.listFailure != message { state.listFailure = message }
                return .none

            case .listed(let proposals, let changes):
                if state.listFailure != nil { state.listFailure = nil }
                let listed = IdentifiedArray(uniqueElements: proposals)
                if listed != state.proposals { state.proposals = listed }
                if changes != state.changes { state.changes = changes }
                return .none

            case .applyTapped(let id):
                return apply(&state, id, title: nil, spec: nil)

            case .editTapped(let id):
                guard let proposal = state.proposals[id: id], proposal.state == .pending else { return .none }
                state.editing = Editing(id: id, title: proposal.title, spec: proposal.spec)
                return .none

            case .editCancelled:
                state.editing = nil
                return .none

            case .editApplied:
                guard let editing = state.editing, let proposal = state.proposals[id: editing.id] else {
                    return .none
                }
                let title = editing.title.trimmingCharacters(in: .whitespacesAndNewlines)
                guard proposal.kind == .specEdit || !title.isEmpty else { return .none }
                // The edit stays open until it's applied, so a failure
                // doesn't lose it.
                return apply(
                    &state, editing.id, title: title == proposal.title ? nil : title,
                    spec: editing.spec == proposal.spec ? nil : editing.spec)

            case .declineTapped(let id):
                guard state.proposals[id: id]?.state == .pending, state.deciding.insert(id).inserted else {
                    return .none
                }
                let session = state.sessionID
                return .run { send in
                    try await daemon.declineProposal(session, id)
                    await send(.decided(id, .declined))
                } catch: { error, send in
                    await send(.failed(id: id, message: error.localizedDescription))
                }

            case .decided(let id, let decision):
                state.deciding.remove(id)
                state.proposals[id: id]?.state = decision
                if state.editing?.id == id { state.editing = nil }
                return .none

            case .startWorkTapped(let item):
                // Keyed by its proposal, which shows it's starting; by the item
                // when the proposal isn't listed yet.
                let key = state.proposals.first(where: { $0.state == .applied(itemID: item) })?.id ?? item
                guard state.deciding.insert(key).inserted else { return .none }
                let session = state.sessionID
                return .run { send in
                    await send(.workStarted(proposal: key, try await daemon.startWork(session, item)))
                } catch: { error, send in
                    await send(.startWorkFailed(proposal: key, message: error.localizedDescription))
                }

            case .workStarted(let proposal, let session):
                state.deciding.remove(proposal)
                return .send(.delegate(.openSession(session)))

            case .startWorkFailed(let proposal, let message):
                state.deciding.remove(proposal)
                state.alert = AlertState {
                    TextState("Couldn’t start work on the task")
                } message: {
                    TextState(message)
                }
                return .none

            case .failed(let id, let message):
                state.deciding.remove(id)
                state.alert = AlertState {
                    TextState("Couldn’t decide on the proposal")
                } message: {
                    TextState(message)
                }
                // Another window may have decided it; show how it stands.
                return .send(.refresh)
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }

    /// What each spec edit among `proposals` changes in its item's spec. One
    /// whose texts are those of the proposal in `known` keeps the change
    /// `kept` has for it, so a refresh works out only what is new.
    private static func changes(
        of proposals: [Proposal], known: IdentifiedArrayOf<Proposal>, kept: [Proposal.ID: ToolCall.Change]
    ) -> [Proposal.ID: ToolCall.Change] {
        var changes: [Proposal.ID: ToolCall.Change] = [:]
        for proposal in proposals {
            guard let current = proposal.currentSpec, !proposal.spec.isEmpty else { continue }
            if let change = kept[proposal.id], let before = known[id: proposal.id],
                before.currentSpec == current, before.spec == proposal.spec
            {
                changes[proposal.id] = change
            } else {
                changes[proposal.id] = ToolCall.Change.between(current, proposal.spec)
            }
        }
        return changes
    }

    private func apply(_ state: inout State, _ id: String, title: String?, spec: String?) -> Effect<Action> {
        guard state.proposals[id: id]?.state == .pending, state.deciding.insert(id).inserted else { return .none }
        let session = state.sessionID
        return .run { send in
            let item = try await daemon.applyProposal(session, id, title, spec)
            await send(.decided(id, .applied(itemID: item.id)))
        } catch: { error, send in
            await send(.failed(id: id, message: error.localizedDescription))
        }
    }
}
