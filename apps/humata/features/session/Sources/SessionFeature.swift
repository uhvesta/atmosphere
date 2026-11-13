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
        /// The specs it reads, for a session in a project.
        case spec
        case terminal
    }

    @ObservableState
    package struct State: Equatable {
        package var session: Session
        /// The view showing.
        package var tab = Tab.chat
        package var review: ReviewFeature.State
        package var docs: DocsFeature.State
        /// The specs it reads.
        package var specs: SpecsFeature.State
        /// Shells in the session's folder, on its host.
        package var terminals: TerminalsFeature.State
        /// What the agent proposed for the board, shown as cards.
        package var proposals: ProposalsFeature.State
        /// Every event but the reply's parts as it streams, oldest first.
        /// Setting it works out what the chat shows once, not on every
        /// render or streamed word.
        package var events: IdentifiedArrayOf<SessionEvent> {
            get { log }
            set {
                log = newValue
                derive()
            }
        }
        private var log: IdentifiedArrayOf<SessionEvent> = []
        /// What the chat shows, oldest first.
        package private(set) var items: [ChatItem] = []
        /// Requests for permission the agent is still waiting on.
        package private(set) var pendingPermissions: [SessionEvent] = []
        /// The failure row offering Try Again: the newest, while the last
        /// turn failed and there's a message to send again.
        package private(set) var retryableFailure: String?
        /// What the session's turns have cost so far.
        package private(set) var usage: Usage?
        /// The reply so far, until its whole message arrives.
        package var streaming = ""
        /// The message being written; it is kept per session, so it is
        /// still there after opening another session or quitting.
        package var draft: String
        @Shared var savedDraft: String
        /// Long text pasted into the box, shown as chips and sent after
        /// what's typed; kept with the draft.
        package var pastes: [Paste]
        @Shared var savedPastes: String
        /// Images put in the box for the next message. Not kept with the
        /// draft: they can be large, and the files they came from are still
        /// there to add again.
        package var images: IdentifiedArrayOf<DraftImage> = []
        /// Whether the session's model can see images; nil until known.
        package var readsImages: Bool?
        /// Small copies of the images sent in the chat, by name.
        package var thumbnails: [String: Data] = [:]
        /// Sent images being read for their thumbnails.
        var loadingThumbnails: Set<String> = []
        /// The thinking the person opened, by item id.
        package var expandedThinking: Set<String> = []
        /// Whether the daemon stopped sending events; watching again resumes.
        package var disconnected = false
        /// Times in a row watching has ended without a new event, which spaces
        /// out the next try.
        package var watchRetries = 0
        /// The person's message being edited, by event id; nil when none is.
        package var editing: String?
        /// The edited message's text.
        package var edited = ""
        /// The queued message being edited in place, by queue id.
        package var editingQueued: String?
        /// Its text as edited so far.
        package var queuedEdit = ""
        @Presents package var alert: AlertState<Never>?
        /// Asks how far to rewind: the chat, or the chat and its files.
        @Presents package var rewindDialog: ConfirmationDialogState<Action.RewindChoice>?
        /// A rewind or its undo is under way.
        package var rewinding = false
        /// The message the last rewind put back in the box, so undoing it
        /// takes it out again if it wasn't changed.
        var rewoundText: String?
        /// The rewind the chat was last read again for, so reading it
        /// again doesn't start over once more.
        var restartedFor: String?
        /// Landing the session, as a sheet.
        @Presents package var land: LandFeature.State?
        /// Merging the base in is under way.
        package var updating = false
        /// The event a search found, shown highlighted with a way to fork
        /// from it; nil when none is.
        package var highlighted: String?
        /// The question or move the agent asked for and is waiting on, from
        /// the inbox.
        package var request: AgentRequest?
        /// The chat row to scroll to once, such as the first proposal waiting.
        package var scrollTarget: String?

        package init(session: Session, highlighted: String? = nil) {
            self.session = session
            self.highlighted = highlighted
            review = ReviewFeature.State(sessionID: session.id, turns: session.turns)
            docs = DocsFeature.State(sessionID: session.id)
            specs = SpecsFeature.State(projectID: session.projectID, itemID: session.itemID)
            terminals = TerminalsFeature.State(place: HostPlace(sessionID: session.id), opensFirst: true)
            proposals = ProposalsFeature.State(sessionID: session.id)
            let saved = Shared(wrappedValue: "", .appStorage(Self.draftKey(session.id)))
            _savedDraft = saved
            draft = saved.wrappedValue
            let pasted = Shared(wrappedValue: "", .appStorage(Self.draftKey(session.id) + "_pastes"))
            _savedPastes = pasted
            pastes = Paste.decode(pasted.wrappedValue)
        }

        /// Whether there's anything to send.
        package var canSend: Bool {
            guard !images.contains(where: \.isPreparing) else { return false }
            return !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !pastes.isEmpty
                || !images.isEmpty
        }

        /// Whether images can go with the next message.
        package var takesImages: Bool { readsImages == true && !session.archived }

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
        package private(set) var queued: [QueuedMessage] = []
        /// Queued messages that never ran because Humata stopped first,
        /// offered back until the person sends again.
        package private(set) var dropped: [QueuedMessage] = []
        /// Dropped messages already put back in the draft.
        package var putBack: Set<String> = [] {
            didSet {
                let putBack = putBack
                dropped.removeAll { putBack.contains($0.id) }
            }
        }

        /// The views it has: a review only for a project's code, and no
        /// terminal once archived, when its folder is gone.
        package var tabs: [Tab] {
            var tabs: [Tab] = session.projectID == nil ? [.chat, .files] : [.chat, .review, .files, .spec]
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

        /// Works out from the events what the chat shows, so views read it
        /// instead of working it out again.
        private mutating func derive() {
            items = ChatItem.items(log)
            usage = Usage.total(log)
            // A request waits until it's answered or its turn ends.
            var waiting: [SessionEvent] = []
            for event in log {
                switch event.kind {
                case .permissionRequested: waiting.append(event)
                case .permissionCompleted(let request, _, _, _):
                    waiting.removeAll { if case .permissionRequested(request, _, _) = $0.kind { true } else { false } }
                case .turnEnded: waiting = []
                default: continue
                }
            }
            pendingPermissions = waiting
            queued = []
            dropped = []
            for event in log {
                switch event.kind {
                case .messageQueued(let id, let text, let images):
                    // Again for one still queued: it was edited, or joined.
                    if let index = queued.firstIndex(where: { $0.id == id }) {
                        queued[index].text = text
                        queued[index].images = images
                    } else {
                        queued.append(QueuedMessage(id: id, text: text, images: images))
                    }
                case .dequeued(let id, let wasDropped):
                    if wasDropped, let message = queued.first(where: { $0.id == id }) { dropped.append(message) }
                    queued.removeAll { $0.id == id }
                case .userMessage: dropped = []
                default: continue
                }
            }
            let putBack = putBack
            dropped.removeAll { putBack.contains($0.id) }
            retryableFailure = nil
            if lastTurnFailed, lastMessage != nil {
                for item in items.reversed() {
                    if case .failure(let id, _) = item {
                        retryableFailure = id
                        break
                    }
                }
            }
        }

        /// Whether the agent is waiting on the person to answer a request.
        package var isWaiting: Bool { !pendingPermissions.isEmpty }

        /// The proposals not decided yet, oldest first.
        package var pendingProposals: [Proposal] {
            proposals.proposals.filter { $0.state == .pending }
        }

        /// The person's last message, to send again after a failed turn.
        package var lastMessage: String? {
            for event in events.reversed() {
                if case .userMessage(let text, _) = event.kind { return text }
            }
            return nil
        }

        /// Whether the last turn failed and nothing has started since.
        package var lastTurnFailed: Bool {
            for event in events.reversed() {
                switch event.kind {
                case .failed: return true
                case .turnStarted, .userMessage: return false
                default: continue
                }
            }
            return false
        }

        /// What's kept of a session looked away from while Humata runs:
        /// the view it showed and the file comments not sent yet.
        package struct Left: Equatable, Sendable {
            package var tab: Tab
            package var drafts: IdentifiedArrayOf<DocsFeature.Draft>
        }

        package var left: Left {
            get { Left(tab: tab, drafts: docs.drafts) }
            set {
                tab = newValue.tab
                docs.drafts = newValue.drafts
            }
        }

        /// Why the turn can't go on yet, such as the model server being
        /// down, until it goes on.
        package var waitReason: String? {
            guard streaming.isEmpty, case .waiting(let reason)? = events.last?.kind else { return nil }
            return reason
        }

        /// Whether `call` is the agent proposing a task or a spec edit.
        func proposes(_ call: String) -> Bool {
            events.contains { event in
                guard case .toolStarted(call, let tool, _) = event.kind else { return false }
                return tool.hasSuffix("propose_task") || tool.hasSuffix("propose_spec_edit")
            }
        }

        mutating func receive(_ event: SessionEvent) {
            receive([event])
        }

        /// Takes in `batch` and works out the chat once, so a history of
        /// thousands of events shows at once instead of one at a time.
        mutating func receive(_ batch: [SessionEvent]) {
            var log = log
            var changed = false
            for event in batch {
                switch event.kind {
                case .assistantDelta(let text):
                    streaming += text
                    continue
                case .assistantMessage, .turnEnded:
                    streaming = ""
                default: break
                }
                log.updateOrAppend(event)
                changed = true
            }
            guard changed else { return }
            events = log
            if batch.contains(where: { $0.kind == .turnStarted }) {
                let started = log.filter { $0.kind == .turnStarted }.count
                review.turns = max(review.turns, started)
            }
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// The view appeared; watches the session until it goes.
        case task
        /// Humata came to the front, so the open reply is seen.
        case becameActive
        case received(SessionEvent)
        /// Events that came together, such as the history when watching
        /// starts.
        case receivedAll([SessionEvent])
        case watchEnded(failure: String?)
        case sendTapped(SendMode)
        /// The message wasn't sent; it goes back in the box if that is empty.
        case sendFailed(text: String, images: [DraftImage], message: String)
        /// The session's model can, or can't, see images.
        case readsImagesFound(Bool)
        /// Image files dropped on the box or chosen.
        case imagesAdded([URL])
        /// An image pasted into the box.
        case imagePasted(Data)
        /// An image in the box is ready to send, or couldn't be read.
        case imagePrepared(id: String, Result<PreparedImage, ImagePrepError>)
        case removeImageTapped(String)
        /// A sent image came into view and needs its thumbnail.
        case thumbnailNeeded(String)
        case thumbnailLoaded(name: String, Data?)
        case abortTapped
        /// Takes the queued message out before its turn starts.
        case unqueueTapped(String)
        /// Takes a pasted chip out of the message.
        case removePasteTapped(String)
        /// Opens the queued message to edit in place.
        case editQueuedTapped(String)
        case editQueuedCancelled
        /// Saves the edit; with nothing left, the message is taken out.
        case editQueuedSaved
        /// Its turn started first, or it was taken out: the edit goes in
        /// the box so it isn't lost.
        case editQueuedFailed(text: String, error: DaemonError?, message: String)
        /// Stops the running turn and puts the person's message back in the
        /// box, to change and send again.
        case stopAndEditTapped
        /// Puts a message that never ran back in the draft.
        case putBackTapped(String)
        /// Sends the person's last message again, after a failed turn.
        case retryTapped
        /// Makes the move the agent asked for, from the strip over the box.
        case moveApproved
        /// Scrolls the chat to the first proposal waiting to be decided.
        case showProposalsTapped
        /// The chat scrolled to the target.
        case scrolled
        /// Asks how far to rewind to before the person's message `event`.
        case rewindTapped(String)
        case rewindDialog(PresentationAction<RewindChoice>)
        /// The chat was cut back; `text` goes back in the box.
        case rewound(String)
        case undoRewindTapped
        case rewindUndone
        case rewindFailed(String)
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
        case specs(SpecsFeature.Action)
        /// Opens another terminal in the session's folder, and shows it.
        case newTerminalTapped
        /// Opens the board at the task it works on.
        case itemTapped
        case terminals(TerminalsFeature.Action)
        case proposals(ProposalsFeature.Action)
        /// Opens the sheet to land the session on a branch, or as pull
        /// requests.
        case landTapped
        case land(PresentationAction<LandFeature.Action>)
        /// Merges each repository's base branch into the session's copy.
        case updateFromBaseTapped
        case updatedFromBase(BaseUpdate)

        package enum RewindChoice: Equatable {
            /// Cut the chat back to before `event`, leaving the code.
            case chat(String)
            /// Put the code back as it was then, too.
            case chatAndFiles(String)
        }

        @CasePathable
        package enum Delegate: Equatable {
            /// A fork was made, or work started on a proposed task; open it.
            case openSession(Session)
            /// Open the board at the task it works on.
            case openItem(projectID: Project.ID, itemID: Item.ID)
            /// Answer the agent's request, as the inbox does.
            case answerRequest(AgentRequest.ID, RequestAnswer)
        }
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.continuousClock) var clock

    @Dependency(\.appActivity) var activity
    @Dependency(\.uuid) var uuid
    private enum CancelID { case watch, seen, activations }

    private func received(_ events: [SessionEvent], _ state: inout State) -> Effect<Action> {
        // A watch sends the history first. Only something new shows the
        // session is live: an offline host's cached history ends its watch
        // at once, and must not be fetched every second.
        let new = events.filter { state.events[id: $0.id] == nil }
        if new.contains(where: \.kind.isLive) { state.watchRetries = 0 }
        // A rewind or its undo replaced the history: read it again from
        // the start, once for each.
        if !state.events.isEmpty,
            let restart = new.last(where: { event in
                switch event.kind {
                case .rewound, .rewindUndone: true
                default: false
                }
            }), restart.id != state.restartedFor
        {
            state.restartedFor = restart.id
            state.events = []
            state.streaming = ""
            return .send(.task)
        }
        state.receive(events)
        var effects: [Effect<Action>] = []
        if new.contains(where: { event in
            if case .toolCompleted(let call, true, _) = event.kind { state.proposes(call) } else { false }
        }) {
            effects.append(.send(.proposals(.refresh)))
        }
        // The agent may have answered the review, or changed the code or
        // the files.
        if new.contains(where: { if case .turnEnded = $0.kind { true } else { false } }) {
            effects.append(markSeen(state))
            switch state.tab {
            case .chat, .terminal: break
            case .review: effects.append(.send(.review(.refresh)))
            case .files: effects.append(.send(.docs(.refresh)))
            case .spec: effects.append(.send(.specs(.refresh)))
            }
        }
        return .merge(effects)
    }

    /// The longest wait before watching again.
    private static let longestRetry = 30

    package init() {}

    /// Says why images can't go in the box, when it's that the model can't
    /// see them.
    private static func refuseImages(_ state: inout State) -> Effect<Action> {
        if state.readsImages == false {
            state.alert = alert(
                "This session’s model can’t see images, so they weren’t added. Describe what’s in them "
                    + "instead, or start a session with a model that reads images, such as Claude Code’s."
            )
        }
        return .none
    }

    /// Finds out once whether the session's model can see images: Claude's
    /// all can; Copilot and model servers say for each model, and a local
    /// model that doesn't say is taken as not.
    private func readsImages(_ state: inout State) -> Effect<Action> {
        guard state.readsImages == nil else { return .none }
        let session = state.session
        switch session.backend {
        case .claude, .scripted:
            state.readsImages = true
            return .none
        case .copilot, .local:
            return .run { send in
                let models = try? await daemon.models(session.host, session.backend, nil)
                let choice = models?.first { $0.id == session.model } ?? (session.model == nil ? models?.first : nil)
                await send(.readsImagesFound(choice?.readsImages ?? (session.backend == .copilot)))
            }
        }
    }

    /// The replies so far leave the inbox, as they're on screen; none
    /// while Humata is behind another app, so the person is told of them.
    private func markSeen(_ state: State) -> Effect<Action> {
        guard activity.isActive() else { return .none }
        let session = state.session.id
        let seen = state.events.filter { if case .turnEnded = $0.kind { true } else { false } }.count
        return .run { _ in try? await daemon.markSeen(session, seen) }
            .cancellable(id: CancelID.seen, cancelInFlight: true)
    }

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Scope(state: \.review, action: \.review) {
            ReviewFeature()
        }
        Scope(state: \.docs, action: \.docs) {
            DocsFeature()
        }
        Scope(state: \.specs, action: \.specs) {
            SpecsFeature()
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
                if let (draft, pasted) = Paste.takeOut(old: state.savedDraft, new: state.draft) {
                    state.draft = draft
                    state.pastes.append(Paste(id: uuid().uuidString, text: pasted))
                    let pastes = Paste.encode(state.pastes)
                    state.$savedPastes.withLock { $0 = pastes }
                }
                state.$savedDraft.withLock { $0 = state.draft }
                return .none

            case .removePasteTapped(let id):
                state.pastes.removeAll { $0.id == id }
                let pastes = Paste.encode(state.pastes)
                state.$savedPastes.withLock { $0 = pastes }
                return .none

            case .proposals(.delegate(.openSession(let session))):
                return .send(.delegate(.openSession(session)))

            case .itemTapped:
                guard let project = state.session.projectID, let item = state.session.itemID else { return .none }
                return .send(.delegate(.openItem(projectID: project, itemID: item)))

            case .specs(.delegate(.openItem(let item))):
                guard let project = state.session.projectID else { return .none }
                return .send(.delegate(.openItem(projectID: project, itemID: item)))

            case .binding, .delegate, .alert, .review, .docs, .specs, .terminals, .proposals, .land:
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
                case .spec:
                    // It may have moved to another item since it opened.
                    state.specs.projectID = state.session.projectID
                    state.specs.itemID = state.session.itemID
                    return .send(.specs(.refresh))
                case .terminal: return .none
                }

            case .task:
                state.disconnected = false
                // The history is sent again, and the reply with it.
                state.streaming = ""
                let session = state.session.id
                return .merge(
                    readsImages(&state),
                    .run { send in
                        // What comes while a batch is shown goes as the
                        // next one, so the history is shown all at once.
                        let batches = EventBatches()
                        let reader = Task {
                            do {
                                for try await event in await daemon.watch(session) { await batches.add(event) }
                                await batches.end(failure: nil)
                            } catch is CancellationError {
                            } catch {
                                await batches.end(failure: error.localizedDescription)
                            }
                        }
                        defer { reader.cancel() }
                        while let batch = await batches.next() {
                            switch batch {
                            case .events(let events): await send(.receivedAll(events))
                            case .ended(let failure):
                                await send(.watchEnded(failure: failure))
                                return
                            }
                        }
                    }
                    .cancellable(id: CancelID.watch, cancelInFlight: true),
                    .run { [activity] send in
                        for await _ in activity.activations() { await send(.becameActive) }
                    }
                    .cancellable(id: CancelID.activations, cancelInFlight: true)
                )

            case .becameActive:
                return markSeen(state)

            case .received(let event):
                return received([event], &state)

            case .receivedAll(let events):
                return received(events, &state)

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
                guard state.canSend else { return .none }
                let text = Paste.message(draft: state.draft, pastes: state.pastes)
                let drafted = Array(state.images)
                let images = drafted.compactMap(\.image)
                state.draft = ""
                state.$savedDraft.withLock { $0 = "" }
                state.pastes = []
                state.$savedPastes.withLock { $0 = "" }
                state.images = []
                let session = state.session.id
                return .run { _ in
                    if images.isEmpty {
                        try await daemon.send(session, text, mode)
                    } else {
                        try await daemon.sendImages(session, text, images, mode)
                    }
                } catch: { error, send in
                    await send(.sendFailed(text: text, images: drafted, message: error.localizedDescription))
                }

            case .sendFailed(let text, let images, let message):
                if state.draft.isEmpty {
                    state.draft = text
                    state.$savedDraft.withLock { $0 = text }
                }
                if state.images.isEmpty { state.images = IdentifiedArray(uniqueElements: images) }
                state.alert = Self.alert(message)
                return .none

            case .readsImagesFound(let reads):
                state.readsImages = reads
                return .none

            case .imagesAdded(let urls):
                guard state.takesImages else { return Self.refuseImages(&state) }
                return .merge(
                    urls.map { url in
                        let id = uuid().uuidString
                        state.images.append(DraftImage(id: id, name: url.lastPathComponent))
                        return .run { send in
                            let prepared = Result { () throws(ImagePrepError) in try ImagePrep.prepare(url) }
                            await send(.imagePrepared(id: id, prepared))
                        }
                    })

            case .imagePasted(let data):
                guard state.takesImages else { return Self.refuseImages(&state) }
                let id = uuid().uuidString
                let name = "Pasted image"
                state.images.append(DraftImage(id: id, name: name))
                return .run { send in
                    let prepared = Result { () throws(ImagePrepError) in try ImagePrep.prepare(data, name: name) }
                    await send(.imagePrepared(id: id, prepared))
                }

            case .imagePrepared(let id, .success(let prepared)):
                // Taken out while it was being prepared.
                guard state.images[id: id] != nil else { return .none }
                state.images[id: id]?.image = prepared.image
                state.images[id: id]?.thumbnail = prepared.thumbnail
                return .none

            case .imagePrepared(let id, .failure(let error)):
                guard state.images.remove(id: id) != nil else { return .none }
                state.alert = Self.alert(error.description)
                return .none

            case .removeImageTapped(let id):
                state.images.remove(id: id)
                return .none

            case .thumbnailNeeded(let name):
                guard state.thumbnails[name] == nil, !state.loadingThumbnails.contains(name) else { return .none }
                state.loadingThumbnails.insert(name)
                let session = state.session.id
                return .run { send in
                    let image = try? await daemon.readImage(session, name)
                    await send(.thumbnailLoaded(name: name, image.flatMap { ImagePrep.thumbnail($0.data) }))
                }

            case .thumbnailLoaded(let name, let thumbnail):
                state.loadingThumbnails.remove(name)
                // An image that can't be read is shown as missing, and isn't
                // read again each time it scrolls into view.
                state.thumbnails[name] = thumbnail ?? Data()
                return .none

            case .abortTapped:
                let session = state.session.id
                return .run { _ in
                    try await daemon.abort(session)
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .retryTapped:
                guard let text = state.lastMessage, !state.isWorking else { return .none }
                let session = state.session.id
                return .run { _ in
                    try await daemon.send(session, text, .enqueue)
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .moveApproved:
                guard let request = state.request, case .move = request.kind else { return .none }
                return .send(.delegate(.answerRequest(request.id, .approve)))

            case .showProposalsTapped:
                state.scrollTarget = state.pendingProposals.first?.id
                return .none

            case .scrolled:
                state.scrollTarget = nil
                return .none

            case .putBackTapped(let id):
                guard let message = state.dropped.first(where: { $0.id == id }) else { return .none }
                state.putBack.insert(id)
                Self.putInDraft(message.text, &state)
                return .none

            case .unqueueTapped(let queued):
                let session = state.session.id
                return .run { _ in
                    try await daemon.unqueue(session, queued)
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .editQueuedTapped(let id):
                guard let message = state.queued.first(where: { $0.id == id }) else { return .none }
                state.editingQueued = id
                state.queuedEdit = message.text
                return .none

            case .editQueuedCancelled:
                state.editingQueued = nil
                state.queuedEdit = ""
                return .none

            case .editQueuedSaved:
                guard let id = state.editingQueued else { return .none }
                let text = state.queuedEdit.trimmingCharacters(in: .whitespacesAndNewlines)
                state.editingQueued = nil
                state.queuedEdit = ""
                let session = state.session.id
                guard !text.isEmpty else { return .send(.unqueueTapped(id)) }
                guard state.queued.first(where: { $0.id == id })?.text != text else { return .none }
                return .run { _ in
                    try await daemon.editQueued(session, id, text)
                } catch: { error, send in
                    await send(
                        .editQueuedFailed(
                            text: text, error: error as? DaemonError, message: error.localizedDescription))
                }

            case .editQueuedFailed(let text, let error, let message):
                Self.putInDraft(text, &state)
                if case .refused(.unknownQueuedMessage, _)? = error {
                    state.alert = AlertState {
                        TextState("Its turn already started")
                    } message: {
                        TextState(
                            """
                            The message left the queue before the edit was saved, so it ran as it was. Your \
                            edit is in the box: send it to follow up, or steer the running turn with it.
                            """)
                    }
                } else {
                    state.alert = Self.alert(message + " Your edit is in the box.")
                }
                return .none

            case .stopAndEditTapped:
                guard state.isWorking, let text = state.lastMessage else { return .none }
                let session = state.session.id
                state.draft = state.draft.isEmpty ? text : text + "\n\n" + state.draft
                state.$savedDraft.withLock { $0 = state.draft }
                return .run { _ in
                    try await daemon.abort(session)
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .rewindTapped(let id):
                guard case .userMessage? = state.events[id: id]?.kind, !state.rewinding else { return .none }
                let files = state.session.projectID != nil && !state.session.archived
                let working = state.isWorking
                state.rewindDialog = ConfirmationDialogState {
                    TextState("Rewind to before this message?")
                } actions: {
                    ButtonState(action: .chat(id)) { TextState("Rewind the Chat") }
                    if files {
                        ButtonState(action: .chatAndFiles(id)) { TextState("Rewind the Chat and Files") }
                    }
                    ButtonState(role: .cancel) { TextState("Cancel") }
                } message: {
                    TextState(
                        (working
                            ? "The agent’s turn stops and queued messages are dropped. "
                            : "")
                            + "Everything after it leaves the chat, and the message goes back in the box to edit. "
                            + (files
                                ? "Rewinding files too puts the code back as it was then, uncommitted changes included. "
                                : "")
                            + "Undo brings it all back until you send another message.")
                }
                return .none

            case .rewindDialog(.presented(let choice)):
                let (before, files) =
                    switch choice {
                    case .chat(let id): (id, false)
                    case .chatAndFiles(let id): (id, true)
                    }
                guard case .userMessage(let text, _)? = state.events[id: before]?.kind else { return .none }
                state.rewinding = true
                let session = state.session.id
                return .run { send in
                    try await daemon.rewind(session, before, files)
                    await send(.rewound(text))
                } catch: { error, send in
                    await send(.rewindFailed(error.localizedDescription))
                }

            case .rewindDialog:
                return .none

            case .rewound(let text):
                state.rewinding = false
                state.rewoundText = text
                state.draft = state.draft.isEmpty ? text : text + "\n\n" + state.draft
                state.$savedDraft.withLock { $0 = state.draft }
                return .none

            case .undoRewindTapped:
                guard !state.rewinding else { return .none }
                state.rewinding = true
                let session = state.session.id
                return .run { send in
                    try await daemon.undoRewind(session)
                    await send(.rewindUndone)
                } catch: { error, send in
                    await send(.rewindFailed(error.localizedDescription))
                }

            case .rewindUndone:
                state.rewinding = false
                if let text = state.rewoundText, state.draft == text {
                    state.draft = ""
                    state.$savedDraft.withLock { $0 = "" }
                }
                state.rewoundText = nil
                return .none

            case .rewindFailed(let message):
                state.rewinding = false
                state.alert = AlertState {
                    TextState("Couldn’t rewind")
                } message: {
                    TextState(
                        "\(message)\n\nTry again once Humata’s daemon answers.")
                }
                return .none

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
                guard case .userMessage(let text, _)? = state.events[id: id]?.kind else { return .none }
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
        .ifLet(\.$rewindDialog, action: \.rewindDialog)
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

    /// Adds `text` to the end of the draft, after a blank line, and keeps it.
    private static func putInDraft(_ text: String, _ state: inout State) {
        state.draft = state.draft.isEmpty ? text : state.draft + "\n\n" + text
        state.$savedDraft.withLock { $0 = state.draft }
    }

    private static func alert(_ message: String) -> AlertState<Never> {
        AlertState {
            TextState("Something went wrong")
        } message: {
            TextState(message)
        }
    }
}

extension SessionEvent.Kind {
    /// Whether it shows the session is live when it's new: a streamed delta
    /// isn't kept, so it's new on every watch, a replayed one too.
    fileprivate var isLive: Bool {
        if case .assistantDelta = self { false } else { true }
    }
}

/// A watch's events, handed on in batches: everything that came since the
/// last batch was taken goes as one.
private actor EventBatches {
    enum Batch {
        case events([SessionEvent])
        /// The watch ended, with why if it failed.
        case ended(failure: String?)
    }

    private var pending: [SessionEvent] = []
    private var ended: Batch?
    private var cancelled = false
    private var waiter: CheckedContinuation<Void, Never>?

    func add(_ event: SessionEvent) {
        pending.append(event)
        wake()
    }

    func end(failure: String?) {
        ended = .ended(failure: failure)
        wake()
    }

    /// The next batch, waiting for one; nil once cancelled.
    func next() async -> Batch? {
        if pending.isEmpty, ended == nil, !cancelled {
            await withTaskCancellationHandler {
                await withCheckedContinuation { waiter = $0 }
            } onCancel: {
                Task { await self.cancel() }
            }
        }
        if cancelled || Task.isCancelled { return nil }
        if !pending.isEmpty {
            defer { pending = [] }
            return .events(pending)
        }
        return ended
    }

    private func cancel() {
        cancelled = true
        wake()
    }

    private func wake() {
        waiter?.resume()
        waiter = nil
    }
}
