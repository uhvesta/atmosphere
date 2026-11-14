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

    /// Identified by its session, so opening another in its place stops
    /// what this one was doing instead of finishing it into the other.
    @ObservableState
    package struct State: Equatable, Identifiable {
        package var id: Session.ID { session.id }
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
                var chat = Chat()
                for event in newValue { chat.add(event) }
                self.chat = chat
                derive()
            }
        }
        private var log: IdentifiedArrayOf<SessionEvent> = []
        /// The chat, worked out an event at a time.
        private var chat = Chat()
        /// What the chat shows, oldest first.
        package var items: [ChatItem] { chat.items }
        /// Whether the history has come since the session opened, so the
        /// chat shows it's loading until then.
        package var historyLoaded = false
        /// How much of a history kept on disk is read, while it's read
        /// before the history comes.
        package var historyLoading: HistoryLoading?
        /// The streamed parts of the reply taken in, so the history sent
        /// again on a reconnect doesn't add them twice.
        var streamed: Set<String> = []
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
        /// Images put in the box for the next message. Kept while Humata
        /// runs, not with the saved draft: they can be large.
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
        /// Why watching ended, when it's more than the connection dropping.
        package var watchFailure: String?
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
        /// The chip it went back as, when it was too long for the field.
        var rewoundPaste: Paste.ID?
        /// The rewind the chat was last read again for, so reading it
        /// again doesn't start over once more.
        var restartedFor: String?
        /// The history is being sent to a chat that opened empty: a rewind
        /// in it happened before, and is no reason to read it again.
        var replaying = false
        /// Landing the session, as a sheet.
        @Presents package var land: LandFeature.State?
        /// Merging the base in is under way.
        package var updating = false
        /// A fork, or an edited message's fork, is being made.
        package var forking = false
        /// Permission requests answered and not yet taken by the agent, so
        /// their buttons can't send a second answer.
        package var answering: Set<String> = []
        /// The last message is being sent again after a failed turn.
        package var retrying = false
        /// Messages sent from the box that the daemon hasn't answered for.
        package var sending = 0
        /// The turn was told to stop and hasn't ended yet.
        package var stopping = false
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
            // Images wait until it's known the model can see them.
            guard images.isEmpty || readsImages == true else { return false }
            return !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !pastes.isEmpty
                || !images.isEmpty
        }

        /// Whether the model is known to see images, so the box offers to
        /// add them.
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

        /// Puts a message that wasn't sent back for its session, which
        /// isn't open: its text and pastes ahead of the draft kept for it,
        /// and its images in `left`, for when it opens again.
        package static func keepUnsent(_ message: Outgoing, in left: inout Left) {
            if !message.typed.isEmpty {
                let draft = Shared(wrappedValue: "", .appStorage(draftKey(message.session)))
                draft.withLock { $0 = $0.isEmpty ? message.typed : message.typed + "\n\n" + $0 }
            }
            if !message.pastes.isEmpty {
                let pastes = Shared(wrappedValue: "", .appStorage(draftKey(message.session) + "_pastes"))
                let since = Paste.decode(pastes.wrappedValue)
                let unsent = message.pastes.filter { paste in !since.contains { $0.id == paste.id } }
                let kept = Paste.encode(unsent + since)
                pastes.withLock { $0 = kept }
            }
            let images = message.images.filter { !$0.isPreparing && left.images[id: $0.id] == nil }
            if !images.isEmpty { left.images = IdentifiedArray(uniqueElements: images + left.images) }
        }

        /// Forgets the draft and pastes kept for `session`, as once it's
        /// deleted.
        package static func forgetDraft(of session: Session.ID) {
            Shared(wrappedValue: "", .appStorage(draftKey(session))).withLock { $0 = "" }
            Shared(wrappedValue: "", .appStorage(draftKey(session) + "_pastes")).withLock { $0 = "" }
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

        /// Whether a turn has started and not yet ended. Kept, as views
        /// read it on every key typed.
        package private(set) var isWorking = false

        /// Takes from the chat what its events add up to, kept there as each
        /// event comes, so views read it instead of working it out again.
        /// Sets only what changed, so views showing the rest aren't drawn
        /// again.
        private mutating func derive() {
            if isWorking != chat.working { isWorking = chat.working }
            let working = isWorking
            if stopping, !working { stopping = false }
            if let land, land.turnRunning != working { self.land?.turnRunning = working }
            if lastMessage != chat.lastMessage { lastMessage = chat.lastMessage }
            if lastTurnFailed != chat.lastTurnFailed { lastTurnFailed = chat.lastTurnFailed }
            if !lastTurnFailed, retrying { retrying = false }
            if usage != chat.usage { usage = chat.usage }
            if pendingPermissions != chat.pendingPermissions { pendingPermissions = chat.pendingPermissions }
            if !answering.isEmpty {
                let ids = Set(
                    pendingPermissions.compactMap { event -> String? in
                        if case .permissionRequested(let id, _, _) = event.kind { id } else { nil }
                    })
                let kept = answering.intersection(ids)
                if kept != answering { answering = kept }
            }
            if queued != chat.queued { queued = chat.queued }
            let putBack = putBack
            let dropped = chat.dropped.filter { !putBack.contains($0.id) }
            if self.dropped != dropped { self.dropped = dropped }
            let retryable = lastTurnFailed && lastMessage != nil ? chat.lastFailure : nil
            if retryableFailure != retryable { retryableFailure = retryable }
        }

        /// Whether the agent is waiting on the person to answer a request.
        package var isWaiting: Bool { !pendingPermissions.isEmpty }

        /// The proposals not decided yet, oldest first.
        package var pendingProposals: [Proposal] {
            proposals.proposals.filter { $0.state == .pending }
        }

        /// The person's last message, to send again after a failed turn.
        package private(set) var lastMessage: String?

        /// Whether the last turn failed and nothing has started since.
        package private(set) var lastTurnFailed = false

        /// How many turns have started and ended in the history.
        var turnsStarted: Int { chat.turnsStarted }
        var turnsEnded: Int { chat.turnsEnded }

        /// What's kept of a session looked away from while Humata runs:
        /// the view it showed, the file it showed with the comment being
        /// written on it and those not sent yet, the review comment half
        /// written, the spec being edited, the message and the queued
        /// message being edited, and the images in the box.
        package struct Left: Equatable, Sendable {
            package var tab: Tab
            var drafts: IdentifiedArrayOf<DocsFeature.Draft>
            /// The file shown, what was picked in it and the comment on it.
            var file: String?
            var picked: DocsFeature.Picked?
            var comment: String
            var review: ReviewFeature.State.Kept
            /// The spec being edited; a save under way isn't kept, as its
            /// answer doesn't come to the session opened again.
            var spec: SpecsFeature.Draft?
            var editing: String?
            var edited: String
            var editingQueued: String?
            var queuedEdit: String
            /// The images in the box that are ready to send. One still
            /// being prepared isn't kept: its result doesn't come to the
            /// session opened again, so it would never be ready.
            var images: IdentifiedArrayOf<DraftImage>
        }

        package var left: Left {
            get {
                var spec = specs.draft
                spec?.saving = false
                return Left(
                    tab: tab, drafts: docs.drafts, file: docs.selected, picked: docs.picked, comment: docs.comment,
                    review: review.kept, spec: spec, editing: editing, edited: edited,
                    editingQueued: editingQueued, queuedEdit: queuedEdit,
                    images: IdentifiedArray(uniqueElements: images.filter { !$0.isPreparing }))
            }
            set {
                tab = newValue.tab
                docs.drafts = newValue.drafts
                docs.selected = newValue.file
                docs.picked = newValue.picked
                docs.comment = newValue.comment
                review.kept = newValue.review
                specs.draft = newValue.spec
                editing = newValue.editing
                edited = newValue.edited
                editingQueued = newValue.editingQueued
                queuedEdit = newValue.queuedEdit
                images = newValue.images
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
            chat.proposals.contains(call)
        }

        /// The tool calls that propose a task or a spec edit.
        var proposingCalls: Set<String> { chat.proposals }

        mutating func receive(_ event: SessionEvent) {
            receive([event])
        }

        /// Takes in `batch` and works out the chat once, so a history of
        /// thousands of events shows at once instead of one at a time.
        mutating func receive(_ batch: [SessionEvent]) {
            // The log is taken out while it's changed and put back once: the
            // state is observed, and changing it where it is copies it whole
            // for each event, which for a long history is seconds.
            var log = self.log
            self.log = []
            var added: [SessionEvent] = []
            var replaced = false
            var stale = false
            var turnEnded = false
            var streaming = self.streaming
            var streamed = self.streamed
            for event in batch {
                switch event.kind {
                case .assistantDelta(let text):
                    if streamed.insert(event.id).inserted { streaming += text }
                    continue
                default: break
                }
                if let known = log[id: event.id] {
                    guard known != event else { continue }
                    log[id: event.id] = event
                    replaced = true
                } else {
                    log.append(event)
                    added.append(event)
                    // Only a reply that's new ends the one being written: a
                    // history sent again holds the replies before it.
                    switch event.kind {
                    case .assistantMessage, .turnEnded:
                        streaming = ""
                        streamed = []
                    default: break
                    }
                    if case .turnEnded = event.kind { turnEnded = true }
                }
                // Code may have changed off the Review tab: it reads again
                // when next shown.
                switch event.kind {
                case .toolCompleted, .turnEnded: stale = true
                default: break
                }
            }
            if stale { review.stale = true }
            // The turn told to stop has ended, even if the next one queued
            // started in the same batch.
            if turnEnded, stopping { stopping = false }
            if streaming != self.streaming { self.streaming = streaming }
            if streamed != self.streamed { self.streamed = streamed }
            guard replaced || !added.isEmpty else {
                self.log = log
                return
            }
            if replaced {
                events = log
            } else {
                self.log = log
                // Live events go on the end of the chat; only a changed
                // event means working it out again. Taken out to change, as
                // the log is.
                var chat = self.chat
                self.chat = Chat()
                for event in added { chat.add(event) }
                self.chat = chat
                derive()
            }
            if batch.contains(where: { $0.kind == .turnStarted }) {
                review.turns = max(review.turns, chat.turnsStarted)
            }
        }
    }

    /// A message handed on to be sent, with what it was made of, so it can
    /// go back in its session's box if it isn't sent.
    package struct Outgoing: Equatable, Sendable {
        package let session: Session.ID
        /// The session's name, to say whose message wasn't sent.
        package let name: String
        /// What was typed, and the chips and images that went with it.
        package let typed: String
        package let pastes: [Paste]
        package let images: [DraftImage]
        package let mode: SendMode
        /// The message as it's sent: what was typed, then each paste.
        package let text: String
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
        /// The history a watch starts with has all come; what comes after
        /// is new.
        case historyReplayed
        /// The history is read from disk first: `read` bytes of `total`.
        case historyLoading(read: UInt64, total: UInt64)
        case watchEnded(failure: String?)
        case sendTapped(SendMode)
        /// A message sent from the box reached the daemon.
        case sent
        /// The message wasn't sent; what was typed, pasted and added goes
        /// back in the box, before anything put there since.
        case sendFailed(text: String, pastes: [Paste], images: [DraftImage], message: String)
        /// The session's model can, or can't, see images.
        case readsImagesFound(Bool)
        /// Image files dropped on the box or chosen.
        case imagesAdded([URL])
        /// An image pasted into the box.
        case imagePasted(Data)
        /// Text added to the end of the box, such as code quoted from a
        /// reply, a file or a diff.
        case quoted(String)
        /// Paste Image was picked with no image on the clipboard.
        case noImageToPaste
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
        /// Copilot's experimental features on or off, from the next turn.
        case experimentalToggled(Bool)
        case experimentalRefused(previous: Bool, message: String)
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
        /// The fork for an edited message was made, but the message
        /// couldn't be sent to it: the fork opens with it in the box.
        case forkSendFailed(fork: Session, text: String)
        case failed(Doing, String)
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

        /// What was being done when a call failed, so the alert names it.
        package enum Doing: Equatable, Sendable {
            case stopping
            case retrying
            case unqueueing
            case forking
            case updating
            case answering(requestID: String)
        }

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
            /// Send the message from the box. Whoever shows the session
            /// sends it, so it's answered for with `sent` or `sendFailed`
            /// while the session is open, and put back in its box after
            /// it's looked away from.
            case send(Outgoing)
        }
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.continuousClock) var clock

    /// How often a reply being written is shown again.
    package static let streamingFrame = Duration.milliseconds(33)

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
        // the start, once for each. One in the history of a chat that
        // opened empty happened before it was read, so that history
        // already is what it left.
        if let restart = new.last(where: { event in
            switch event.kind {
            case .rewound, .rewindUndone: true
            default: false
            }
        }), restart.id != state.restartedFor {
            state.restartedFor = restart.id
            if !state.replaying, !state.events.isEmpty {
                state.events = []
                state.streaming = ""
                state.streamed = []
                return .send(.task)
            }
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
                "Images weren’t added",
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
        let seen = state.turnsEnded
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

            case .quoted(let text):
                let draft = state.draft.trimmingCharacters(in: .whitespacesAndNewlines)
                state.draft = draft.isEmpty ? text + "\n\n" : state.draft + "\n\n" + text + "\n\n"
                state.$savedDraft.withLock { $0 = state.draft }
                return .none

            case .review(.delegate(.quote(let text))):
                return .send(.quoted(text))

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
                guard state.ships, !state.isWorking else { return .none }
                state.land = LandFeature.State(sessionID: state.session.id, title: state.session.name)
                return .none

            case .updateFromBaseTapped:
                // Merging into files the agent is editing would mix the two.
                guard state.ships, !state.updating, !state.isWorking else { return .none }
                state.updating = true
                let session = state.session.id
                return .run { send in
                    await send(.updatedFromBase(try await daemon.updateFromBase(session)))
                } catch: { error, send in
                    await send(.failed(.updating, error.localizedDescription))
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
                // What was typed in a terminal may have changed the code.
                if state.tab == .terminal, tab != .terminal { state.review.stale = true }
                state.tab = tab
                switch tab {
                // Review and Files read again as their views appear.
                case .chat, .review, .files: return .none
                case .spec:
                    // It may have moved to another item since it opened.
                    state.specs.projectID = state.session.projectID
                    state.specs.itemID = state.session.itemID
                    return .send(.specs(.refresh))
                case .terminal: return .none
                }

            case .task:
                state.disconnected = false
                state.watchFailure = nil
                state.replaying = state.events.isEmpty
                // The history is sent again, and the reply's parts with it;
                // those already shown are skipped, so the reply stays.
                let session = state.session.id
                // A chat opened again, or a watch started over, has the
                // history up to its last event; only what follows is sent.
                let after = state.events.last?.id
                return .merge(
                    readsImages(&state),
                    .run { send in
                        // The history goes as one batch once it has all
                        // come; after it, what comes while a batch is shown
                        // goes as the next one.
                        let batches = EventBatches()
                        let reader = Task {
                            do {
                                for try await item in await daemon.watch(session, after) {
                                    switch item {
                                    case .event(let event): await batches.add(event)
                                    case .loading(let read, let total):
                                        await send(.historyLoading(read: read, total: total))
                                    case .historyLoaded: await batches.historyLoaded()
                                    }
                                }
                                await batches.end(failure: nil)
                            } catch is CancellationError {
                            } catch DaemonError.disconnected, DaemonError.notRunning {
                                // A plain drop: the notice says so already.
                                await batches.end(failure: nil)
                            } catch {
                                await batches.end(failure: error.localizedDescription)
                            }
                        }
                        defer { reader.cancel() }
                        while let batch = await batches.next() {
                            switch batch {
                            case .events(let events, let replayed):
                                await send(.receivedAll(events))
                                if replayed { await send(.historyReplayed) }
                                // A reply comes a few words at a time, and
                                // each batch lays the chat out again: what
                                // streams in is taken at most once a frame,
                                // the words since in the next batch.
                                if !replayed, events.contains(where: { !$0.kind.isLive }) {
                                    try? await clock.sleep(for: Self.streamingFrame)
                                }
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
                state.historyLoaded = true
                state.historyLoading = nil
                // An empty history's end, or one said again: nothing to show.
                guard !events.isEmpty else { return .none }
                return received(events, &state)

            case .historyReplayed:
                state.replaying = false
                return .none

            case .historyLoading(let read, let total):
                state.historyLoading = HistoryLoading(read: read, total: total)
                return .none

            case .watchEnded(let failure):
                state.replaying = false
                state.historyLoaded = true
                state.historyLoading = nil
                state.watchFailure = failure
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
                let outgoing = Outgoing(
                    session: state.session.id, name: state.session.name, typed: state.draft, pastes: state.pastes,
                    images: Array(state.images), mode: mode,
                    text: Paste.message(draft: state.draft, pastes: state.pastes))
                state.draft = ""
                state.$savedDraft.withLock { $0 = "" }
                state.pastes = []
                state.$savedPastes.withLock { $0 = "" }
                state.images = []
                state.sending += 1
                return .send(.delegate(.send(outgoing)))

            case .sent:
                if state.sending > 0 { state.sending -= 1 }
                return .none

            case .sendFailed(let text, let pastes, let images, let message):
                if state.sending > 0 { state.sending -= 1 }
                // Ahead of what was typed and added since, in the order
                // it would have been sent.
                if !text.isEmpty {
                    state.draft = state.draft.isEmpty ? text : text + "\n\n" + state.draft
                    state.$savedDraft.withLock { $0 = state.draft }
                }
                let failedPastes = pastes.filter { paste in !state.pastes.contains { $0.id == paste.id } }
                if !failedPastes.isEmpty {
                    state.pastes = failedPastes + state.pastes
                    let kept = Paste.encode(state.pastes)
                    state.$savedPastes.withLock { $0 = kept }
                }
                let failedImages = images.filter { state.images[id: $0.id] == nil }
                if !failedImages.isEmpty {
                    state.images = IdentifiedArray(uniqueElements: failedImages + state.images)
                }
                state.alert = Self.alert("Couldn’t send the message", message)
                return .none

            case .readsImagesFound(let reads):
                state.readsImages = reads
                // Those added before it was known come out, with why.
                guard !reads, !state.images.isEmpty else { return .none }
                state.images = []
                return Self.refuseImages(&state)

            case .imagesAdded(let urls):
                // Taken until the model is known not to see them.
                guard state.readsImages != false, !state.session.archived else {
                    return Self.refuseImages(&state)
                }
                return .merge(
                    urls.map { url in
                        let id = uuid().uuidString
                        state.images.append(DraftImage(id: id, name: url.lastPathComponent))
                        return .run { send in
                            let prepared = Result { () throws(ImagePrepError) in try ImagePrep.prepare(url) }
                            await send(.imagePrepared(id: id, prepared))
                        }
                    })

            case .noImageToPaste:
                state.alert = Self.alert(
                    "No image to paste",
                    "The clipboard has no image. Copy one, or take a screenshot to the clipboard with ⌃⇧⌘4, "
                        + "then paste again.")
                return .none

            case .imagePasted(let data):
                guard state.readsImages != false, !state.session.archived else {
                    return Self.refuseImages(&state)
                }
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
                state.alert = Self.alert("Couldn’t add the image", error.description)
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
                guard !state.stopping else { return .none }
                // Shown until the turn ends; with none under way, nothing
                // would end it.
                if state.isWorking { state.stopping = true }
                let session = state.session.id
                return .run { _ in
                    try await daemon.abort(session)
                } catch: { error, send in
                    await send(.failed(.stopping, error.localizedDescription))
                }

            case .retryTapped:
                guard let text = state.lastMessage, !state.isWorking, !state.retrying else { return .none }
                state.retrying = true
                let session = state.session.id
                return .run { _ in
                    try await daemon.send(session, text, .enqueue)
                } catch: { error, send in
                    await send(.failed(.retrying, error.localizedDescription))
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
                putInBox(message.text, ahead: false, &state)
                return .none

            case .unqueueTapped(let queued):
                let session = state.session.id
                return .run { _ in
                    try await daemon.unqueue(session, queued)
                } catch: { error, send in
                    await send(.failed(.unqueueing, error.localizedDescription))
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
                putInBox(text, ahead: false, &state)
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
                    state.alert = Self.alert("Couldn’t save the edit", message + " Your edit is in the box.")
                }
                return .none

            case .stopAndEditTapped:
                guard state.isWorking, !state.stopping, let text = state.lastMessage else { return .none }
                state.stopping = true
                let session = state.session.id
                putInBox(text, ahead: true, &state)
                return .run { _ in
                    try await daemon.abort(session)
                } catch: { error, send in
                    await send(.failed(.stopping, error.localizedDescription))
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
                                ? "Rewinding files too puts the code back as it was then, "
                                    + "uncommitted changes included. "
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
                let paste = putInBox(text, ahead: true, &state)
                state.rewoundPaste = paste
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
                if let paste = state.rewoundPaste {
                    if state.pastes.contains(where: { $0.id == paste }) {
                        state.pastes.removeAll { $0.id == paste }
                        let pastes = Paste.encode(state.pastes)
                        state.$savedPastes.withLock { $0 = pastes }
                    }
                } else if let text = state.rewoundText, state.draft == text {
                    state.draft = ""
                    state.$savedDraft.withLock { $0 = "" }
                }
                state.rewoundText = nil
                state.rewoundPaste = nil
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
                guard state.answering.insert(request).inserted else { return .none }
                let session = state.session.id
                return .run { _ in
                    try await daemon.approve(session, request, approved)
                } catch: { error, send in
                    await send(.failed(.answering(requestID: request), error.localizedDescription))
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
                state.alert = Self.alert("Couldn’t change what the agent may do", message)
                return .none

            case .experimentalToggled(let on):
                let previous = state.session.experimental
                guard on != previous else { return .none }
                state.session.experimental = on
                let session = state.session.id
                return .run { _ in
                    _ = try await daemon.update(session, SessionUpdate(experimental: on))
                } catch: { error, send in
                    await send(.experimentalRefused(previous: previous, message: error.localizedDescription))
                }

            case .experimentalRefused(let previous, let message):
                state.session.experimental = previous
                state.alert = Self.alert("Couldn’t change Copilot’s experimental features", message)
                return .none

            case .thinkingToggled(let id):
                state.expandedThinking.formSymmetricDifference([id])
                return .none

            case .forkTapped(let before, let chatOnly):
                guard !state.forking else { return .none }
                state.forking = true
                let session = state.session
                return .run { send in
                    await send(.forked(try await daemon.fork(session.id, before, "\(session.name) fork", chatOnly)))
                } catch: { error, send in
                    await send(.failed(.forking, error.localizedDescription))
                }

            case .forkFromHighlightTapped:
                guard state.highlighted != nil, !state.forking else { return .none }
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
                guard let before = state.editing, !text.isEmpty, !state.forking else { return .none }
                // The edit stays open until the fork is made, so a failure
                // doesn't lose it.
                state.forking = true
                let session = state.session
                return .run { send in
                    let fork = try await daemon.fork(session.id, before, "\(session.name) edit", false)
                    do {
                        try await daemon.send(fork.id, text, .enqueue)
                    } catch {
                        await send(.forkSendFailed(fork: fork, text: text))
                        return
                    }
                    await send(.forked(fork))
                } catch: { error, send in
                    await send(.failed(.forking, error.localizedDescription))
                }

            case .forked(let fork):
                state.forking = false
                state.editing = nil
                state.edited = ""
                return .send(.delegate(.openSession(fork)))

            case .forkSendFailed(let fork, let text):
                // The fork's box keeps the text, to send from there.
                let draft = Shared(wrappedValue: "", .appStorage(State.draftKey(fork.id)))
                draft.withLock { $0 = $0.isEmpty ? text : $0 + "\n\n" + text }
                return .send(.forked(fork))

            case .failed(let doing, let message):
                switch doing {
                case .updating: state.updating = false
                case .forking: state.forking = false
                case .retrying: state.retrying = false
                case .answering(let request): state.answering.remove(request)
                case .stopping: state.stopping = false
                case .unqueueing: break
                }
                state.alert = Self.alert(Self.title(doing), message)
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

    /// Puts `text` in the box and keeps it: ahead of what's typed or after
    /// it, a blank line apart. Text too long to edit in the field goes back
    /// as a chip instead, whose id this gives.
    @discardableResult
    private func putInBox(_ text: String, ahead: Bool, _ state: inout State) -> Paste.ID? {
        guard text.utf8.count <= Paste.putBackBytes else {
            let paste = Paste(id: uuid().uuidString, text: text)
            state.pastes.insert(paste, at: ahead ? 0 : state.pastes.count)
            let pastes = Paste.encode(state.pastes)
            state.$savedPastes.withLock { $0 = pastes }
            return paste.id
        }
        if state.draft.isEmpty {
            state.draft = text
        } else {
            state.draft = ahead ? text + "\n\n" + state.draft : state.draft + "\n\n" + text
        }
        state.$savedDraft.withLock { $0 = state.draft }
        return nil
    }

    private static func title(_ doing: Action.Doing) -> String {
        switch doing {
        case .stopping: "Couldn’t stop the turn"
        case .retrying: "Couldn’t send your message again"
        case .unqueueing: "Couldn’t take the message out of the queue"
        case .forking: "Couldn’t fork the session"
        case .updating: "Couldn’t update from the base branch"
        case .answering: "Couldn’t answer the permission request"
        }
    }

    private static func alert(_ title: String, _ message: String) -> AlertState<Never> {
        AlertState {
            TextState(title)
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

/// A watch's events, handed on in batches: the history it starts with as
/// one, then everything that came since the last batch was taken.
private actor EventBatches {
    enum Batch {
        /// `replayed` when they end the history the watch starts with.
        case events([SessionEvent], replayed: Bool)
        /// The watch ended, with why if it failed.
        case ended(failure: String?)
    }

    private var pending: [SessionEvent] = []
    /// How many of the pending events the history ends after, when it's
    /// all sent and no batch has said so yet: the next one holds those and
    /// says so, even with no events.
    private var historyEnd: Int?
    /// Whether the history the watch starts with has been handed on. Until
    /// then its events wait for its end, however they arrive.
    private var historyDone = false
    private var ended: Batch?
    private var cancelled = false
    private var waiter: CheckedContinuation<Void, Never>?

    func add(_ event: SessionEvent) {
        pending.append(event)
        wake()
    }

    /// The history's events have all been added, if it had any.
    func historyLoaded() {
        historyEnd = pending.count
        wake()
    }

    func end(failure: String?) {
        ended = .ended(failure: failure)
        wake()
    }

    /// The next batch, waiting for one; nil once cancelled. A watch that
    /// ends before its history has all come gives what did come.
    func next() async -> Batch? {
        // Each event added wakes it, so it looks again before going on.
        while !cancelled, ended == nil, historyEnd == nil, !historyDone || pending.isEmpty {
            await withTaskCancellationHandler {
                await withCheckedContinuation { waiter = $0 }
            } onCancel: {
                Task { await self.cancel() }
            }
        }
        if cancelled || Task.isCancelled { return nil }
        if let end = historyEnd {
            // What came after the history's end is new, and goes next.
            let history = Array(pending.prefix(end))
            pending.removeFirst(min(end, pending.count))
            historyEnd = nil
            historyDone = true
            return .events(history, replayed: true)
        }
        if !pending.isEmpty {
            defer { pending = [] }
            return .events(pending, replayed: false)
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

/// How much of a session's history kept on disk is read as it opens.
package struct HistoryLoading: Equatable, Sendable {
    package var read: UInt64
    /// 0 when its size isn't known.
    package var total: UInt64

    /// How far along it is, or nil when its size isn't known.
    package var fraction: Double? {
        guard total > 0 else { return nil }
        return min(Double(read) / Double(total), 1)
    }
}
