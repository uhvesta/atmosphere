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

        package init(session: Session) {
            self.session = session
            review = ReviewFeature.State(sessionID: session.id, turns: session.turns)
            docs = DocsFeature.State(sessionID: session.id)
            terminals = TerminalsFeature.State(place: HostPlace(sessionID: session.id), opensFirst: true)
            let saved = Shared(wrappedValue: "", .appStorage(Self.draftKey(session.id)))
            _savedDraft = saved
            draft = saved.wrappedValue
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

        @CasePathable
        package enum Delegate: Equatable {
            /// A fork was made; open it.
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
        Reduce { state, action in
            switch action {
            case .binding(\.draft):
                state.$savedDraft.withLock { $0 = state.draft }
                return .none

            case .binding, .delegate, .alert, .review, .docs, .terminals:
                return .none

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
                state.alert = Self.alert(message)
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }

    private static func alert(_ message: String) -> AlertState<Never> {
        AlertState {
            TextState("Something went wrong")
        } message: {
            TextState(message)
        }
    }
}
