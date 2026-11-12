import ComposableArchitecture
import Foundation
import HumataDaemonClient
import HumataDocs
import HumataReview
import HumataTerminal

/// One session: its events as they happen, a message to send, permissions
/// to answer and what the session may do without asking, and forks, with
/// the code or chat only, including editing an earlier message; its
/// changes, to review; and the files in its folder, to read and comment on.
@Reducer
package struct SessionFeature {
    package enum Tab: Hashable, Sendable {
        case chat
        case review
        case files
        case terminal
    }

    @ObservableState
    package struct State: Equatable {
        package var session: Session
        package var tab = Tab.chat
        package var review: ReviewFeature.State
        package var docs: DocsFeature.State
        /// Shells in the session's folder, on its host.
        package var terminals: TerminalsFeature.State
        /// What the agent proposed for the board, shown as cards.
        package var proposals: ProposalsFeature.State
        /// Every event but the reply's parts as it streams, oldest first.
        package var events: IdentifiedArrayOf<SessionEvent> = []
        /// The reply so far, until its whole message arrives.
        package var streaming = ""
        /// The message being written; it is kept per session, so it is
        /// still there after opening another session or quitting.
        package var draft: String
        @Shared var savedDraft: String
        /// The thinking the person opened, by item id.
        package var expandedThinking: Set<String> = []
        /// Whether the daemon stopped sending events; watching again resumes.
        package var disconnected = false
        /// Times in a row watching has ended without an event, which spaces
        /// out the next try.
        var watchRetries = 0
        /// The person's message being edited, by event id; nil when none is.
        package var editing: String?
        /// The edited message's text.
        package var edited = ""
        @Presents package var alert: AlertState<Never>?
        /// Landing the session, as a sheet.
        @Presents package var land: LandFeature.State?
        /// Merging the base in is under way.
        package var updating = false
        /// The event a search found, shown highlighted with a way to fork
        /// from it; nil when none is.
        package var highlighted: String?

        package init(session: Session, highlighted: String? = nil) {
            self.session = session
            self.highlighted = highlighted
            review = ReviewFeature.State(sessionID: session.id, turns: session.turns)
            docs = DocsFeature.State(sessionID: session.id)
            terminals = TerminalsFeature.State(place: HostPlace(sessionID: session.id), opensFirst: true)
            proposals = ProposalsFeature.State(sessionID: session.id)
            let saved = Shared(wrappedValue: "", .appStorage(Self.draftKey(session.id)))
            _savedDraft = saved
            draft = saved.wrappedValue
        }

        /// The chat row of the highlighted event, once the history has it:
        /// a tool's row for its call or output, otherwise the event's own.
        package var highlightedItem: String? {
            guard let highlighted, let event = events[id: highlighted] else { return nil }
            switch event.kind {
            case .toolStarted(let callID, _, _), .toolCompleted(let callID, _, _): return callID
            default: return event.id
            }
        }

        /// Where a fork from the highlighted event starts: before the first
        /// event after its turn ended, so the fork keeps that whole turn; nil
        /// to keep everything, when its turn is the last.
        package var forkPointAfterHighlight: String? {
            guard let highlighted, let index = events.index(id: highlighted) else { return nil }
            let rest = events.elements[index...]
            guard let ended = rest.firstIndex(where: { if case .turnEnded = $0.kind { true } else { false } }),
                ended + 1 < events.count
            else { return nil }
            return events.elements[ended + 1].id
        }

        /// Where the session's draft is kept; storage keys can't hold dots.
        static func draftKey(_ session: String) -> String {
            "draft_" + session.replacingOccurrences(of: ".", with: "_")
        }

        /// Messages sent while a turn ran, waiting for their own turn,
        /// first to run first.
        package var queued: [QueuedMessage] {
            var queued: [QueuedMessage] = []
            for event in events {
                switch event.kind {
                case .messageQueued(let id, let text): queued.append(QueuedMessage(id: id, text: text))
                case .dequeued(let id): queued.removeAll { $0.id == id }
                default: continue
                }
            }
            return queued
        }

        /// The views it has: a review only for a project's code, and no
        /// terminal once archived, when its folder is gone.
        package var tabs: [Tab] {
            var tabs: [Tab] = session.projectID == nil ? [.chat, .files] : [.chat, .review, .files]
            if !session.archived { tabs.append(.terminal) }
            return tabs
        }

        /// Whether it has code of a project's to land or update: not a
        /// planning session, and not archived.
        package var ships: Bool {
            session.projectID != nil && !session.planning && !session.archived
        }

        /// Whether a turn has started and not yet ended.
        package var isWorking: Bool {
            for event in events.reversed() {
                switch event.kind {
                case .turnStarted: return true
                case .turnEnded, .idle: return false
                default: continue
                }
            }
            return false
        }

        /// What the chat shows, oldest first.
        package var items: [ChatItem] { ChatItem.items(events) }

        package var usage: Usage? { Usage.total(events) }

        /// Whether the agent is waiting on the person to answer a request.
        package var isWaiting: Bool { !pendingPermissions.isEmpty }

        /// Why the turn can't go on yet, such as the model server being
        /// down, until it goes on.
        package var waitReason: String? {
            guard streaming.isEmpty, case .waiting(let reason)? = events.last?.kind else { return nil }
            return reason
        }

        /// Requests for permission the agent is still waiting on.
        package var pendingPermissions: [SessionEvent] {
            let answered = Set(
                events.compactMap { event -> String? in
                    guard case .permissionCompleted(let request, _, _) = event.kind else { return nil }
                    return request
                })
            return events.filter { event in
                guard case .permissionRequested(let request, _, _) = event.kind else { return false }
                return !answered.contains(request)
            }
        }

        /// Whether `call` is the agent proposing a task or a spec edit.
        func proposes(_ call: String) -> Bool {
            events.contains { event in
                guard case .toolStarted(call, let tool, _) = event.kind else { return false }
                return tool.hasSuffix("propose_task") || tool.hasSuffix("propose_spec_edit")
            }
        }

        mutating func receive(_ event: SessionEvent) {
            defer {
                if case .turnStarted = event.kind {
                    let started = events.filter { $0.kind == .turnStarted }.count
                    review.turns = max(review.turns, started)
                }
            }
            switch event.kind {
            case .assistantDelta(let text):
                streaming += text
            case .assistantMessage, .turnEnded:
                streaming = ""
                events.updateOrAppend(event)
            default:
                events.updateOrAppend(event)
            }
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// The view appeared; watches the session until it goes.
        case task
        case received(SessionEvent)
        case watchEnded(failure: String?)
        case sendTapped(SendMode)
        /// The message wasn't sent; it goes back in the box if that is empty.
        case sendFailed(text: String, message: String)
        case abortTapped
        /// Takes the queued message out before its turn starts.
        case unqueueTapped(String)
        case answerPermission(requestID: String, approved: Bool)
        case approvalModePicked(ApprovalMode)
        /// The daemon refused the new mode, so the session keeps `previous`.
        case approvalModeRefused(previous: ApprovalMode, message: String)
        case thinkingToggled(id: String)
        /// Forks the session before `event`, or with all of it if nil. A
        /// chat-only fork keeps working in this session's copy of the code.
        case forkTapped(before: String?, chatOnly: Bool)
        /// Forks with everything up to the end of the highlighted event's
        /// turn, with the code as it was then.
        case forkFromHighlightTapped
        case highlightDismissed
        /// Starts editing the person's message `event`.
        case editTapped(String)
        case editCancelled
        /// Forks before the edited message, with the code as it was, and
        /// sends the fork the edited text.
        case editSent
        case forked(Session)
        case failed(String)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)
        case tabPicked(Tab)
        case review(ReviewFeature.Action)
        case docs(DocsFeature.Action)
        /// Opens another terminal in the session's folder, and shows it.
        case newTerminalTapped
        case terminals(TerminalsFeature.Action)
        case proposals(ProposalsFeature.Action)
        /// Opens the sheet to land the session on a branch, or as pull
        /// requests.
        case landTapped
        case land(PresentationAction<LandFeature.Action>)
        /// Merges each repository's base branch into the session's copy.
        case updateFromBaseTapped
        case updatedFromBase(BaseUpdate)

        @CasePathable
        package enum Delegate: Equatable {
            /// A fork was made, or work started on a proposed task; open it.
            case openSession(Session)
        }
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.continuousClock) var clock

    private enum CancelID { case watch, seen }

    /// The longest wait before watching again.
    private static let longestRetry = 30

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Scope(state: \.review, action: \.review) {
            ReviewFeature()
        }
        Scope(state: \.docs, action: \.docs) {
            DocsFeature()
        }
        Scope(state: \.terminals, action: \.terminals) {
            TerminalsFeature()
        }
        Scope(state: \.proposals, action: \.proposals) {
            ProposalsFeature()
        }
        Reduce { state, action in
            switch action {
            case .binding(\.draft):
                state.$savedDraft.withLock { $0 = state.draft }
                return .none

            case .proposals(.delegate(.openSession(let session))):
                return .send(.delegate(.openSession(session)))

            case .binding, .delegate, .alert, .review, .docs, .terminals, .proposals, .land:
                return .none

            case .landTapped:
                guard state.ships else { return .none }
                state.land = LandFeature.State(sessionID: state.session.id, title: state.session.name)
                return .none

            case .updateFromBaseTapped:
                guard state.ships, !state.updating else { return .none }
                state.updating = true
                let session = state.session.id
                return .run { send in
                    await send(.updatedFromBase(try await daemon.updateFromBase(session)))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .updatedFromBase(let update):
                state.updating = false
                state.alert = Self.updated(update)
                return update.repositories.contains { $0.outcome == .merged } ? .send(.review(.refresh)) : .none

            case .newTerminalTapped:
                guard state.tabs.contains(.terminal) else { return .none }
                state.tab = .terminal
                // Shown for the first time, the tab opens one itself.
                return state.terminals.listed ? .send(.terminals(.newTapped)) : .none

            case .tabPicked(let tab):
                state.tab = tab
                switch tab {
                case .chat: return .none
                case .review: return .send(.review(.refresh))
                case .files: return .send(.docs(.refresh))
                case .terminal: return .none
                }

            case .task:
                state.disconnected = false
                // The history is sent again, and the reply with it.
                state.streaming = ""
                let session = state.session.id
                return .run { send in
                    do {
                        for try await event in await daemon.watch(session) {
                            await send(.received(event))
                        }
                        await send(.watchEnded(failure: nil))
                    } catch is CancellationError {
                    } catch {
                        await send(.watchEnded(failure: error.localizedDescription))
                    }
                }
                .cancellable(id: CancelID.watch, cancelInFlight: true)

            case .received(let event):
                state.watchRetries = 0
                state.receive(event)
                if case .toolCompleted(let call, true, _) = event.kind, state.proposes(call) {
                    return .send(.proposals(.refresh))
                }
                // The agent may have answered the review, or changed the code
                // or the files.
                guard case .turnEnded = event.kind else { return .none }
                // The reply is on screen, so it leaves the inbox.
                let session = state.session.id
                let seen = state.events.filter { if case .turnEnded = $0.kind { true } else { false } }.count
                let markSeen: Effect<Action> = .run { _ in try? await daemon.markSeen(session, seen) }
                    .cancellable(id: CancelID.seen, cancelInFlight: true)
                switch state.tab {
                case .chat: return markSeen
                case .review: return .merge(markSeen, .send(.review(.refresh)))
                case .files: return .merge(markSeen, .send(.docs(.refresh)))
                case .terminal: return markSeen
                }

            case .watchEnded:
                // The daemon restarted or the host dropped; watch again, less
                // often the longer it stays away. The notice says so and
                // lets the person try at once.
                state.disconnected = true
                let wait = min(1 << min(state.watchRetries, 5), Self.longestRetry)
                state.watchRetries += 1
                return .run { send in
                    try await clock.sleep(for: .seconds(wait))
                    await send(.task)
                }
                .cancellable(id: CancelID.watch, cancelInFlight: true)

            case .sendTapped(let mode):
                let text = state.draft.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !text.isEmpty else { return .none }
                state.draft = ""
                state.$savedDraft.withLock { $0 = "" }
                let session = state.session.id
                return .run { _ in
                    try await daemon.send(session, text, mode)
                } catch: { error, send in
                    await send(.sendFailed(text: text, message: error.localizedDescription))
                }

            case .sendFailed(let text, let message):
                if state.draft.isEmpty {
                    state.draft = text
                    state.$savedDraft.withLock { $0 = text }
                }
                state.alert = Self.alert(message)
                return .none

            case .abortTapped:
                let session = state.session.id
                return .run { _ in
                    try await daemon.abort(session)
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .unqueueTapped(let queued):
                let session = state.session.id
                return .run { _ in
                    try await daemon.unqueue(session, queued)
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .answerPermission(let request, let approved):
                let session = state.session.id
                return .run { _ in
                    try await daemon.approve(session, request, approved)
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .approvalModePicked(let mode):
                let previous = state.session.approvalMode
                guard mode != previous else { return .none }
                state.session.approvalMode = mode
                let session = state.session.id
                return .run { _ in
                    try await daemon.setApprovalMode(session, mode)
                } catch: { error, send in
                    await send(.approvalModeRefused(previous: previous, message: error.localizedDescription))
                }

            case .approvalModeRefused(let previous, let message):
                state.session.approvalMode = previous
                state.alert = Self.alert(message)
                return .none

            case .thinkingToggled(let id):
                state.expandedThinking.formSymmetricDifference([id])
                return .none

            case .forkTapped(let before, let chatOnly):
                let session = state.session
                return .run { send in
                    await send(.forked(try await daemon.fork(session.id, before, "\(session.name) fork", chatOnly)))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .forkFromHighlightTapped:
                guard state.highlighted != nil else { return .none }
                return .send(.forkTapped(before: state.forkPointAfterHighlight, chatOnly: false))

            case .highlightDismissed:
                state.highlighted = nil
                return .none

            case .editTapped(let id):
                guard case .userMessage(let text)? = state.events[id: id]?.kind else { return .none }
                state.editing = id
                state.edited = text
                return .none

            case .editCancelled:
                state.editing = nil
                state.edited = ""
                return .none

            case .editSent:
                let text = state.edited.trimmingCharacters(in: .whitespacesAndNewlines)
                guard let before = state.editing, !text.isEmpty else { return .none }
                state.editing = nil
                state.edited = ""
                let session = state.session
                return .run { send in
                    let fork = try await daemon.fork(session.id, before, "\(session.name) edit", false)
                    try await daemon.send(fork.id, text, .enqueue)
                    await send(.forked(fork))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .forked(let fork):
                return .send(.delegate(.openSession(fork)))

            case .failed(let message):
                state.updating = false
                state.alert = Self.alert(message)
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
        .ifLet(\.$land, action: \.land) {
            LandFeature()
        }
    }

    /// How merging the base in went, repository by repository.
    private static func updated(_ update: BaseUpdate) -> AlertState<Never> {
        let lines = update.repositories.map { repository in
            switch repository.outcome {
            case .upToDate: "\(repository.name): already up to date with \(repository.upstream)."
            case .merged: "\(repository.name): merged \(repository.upstream) in."
            case .conflicts:
                "\(repository.name): conflicts with \(repository.upstream) in "
                    + repository.conflicts.joined(separator: ", ") + "."
            }
        }
        let asked = update.sentToAgent ? ["The agent was asked to resolve the conflicts."] : []
        let conflicted = update.repositories.contains { $0.outcome == .conflicts }
        return AlertState {
            TextState(conflicted ? "Merged with Conflicts" : "Updated from the Base Branch")
        } message: {
            TextState((lines + asked).joined(separator: "\n"))
        }
    }

    private static func alert(_ message: String) -> AlertState<Never> {
        AlertState {
            TextState("Something went wrong")
        } message: {
            TextState(message)
        }
    }
}
