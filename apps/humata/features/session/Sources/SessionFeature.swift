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

        /// Its name in the Session menu.
        package var menuTitle: String {
            switch self {
            case .chat: "Chat"
            case .review: "Review"
            case .files: "Files"
            case .spec: "Spec"
            case .terminal: "Terminal"
            }
        }
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
        /// Its project's plan, beside the chat.
        package var planPanel: PlanPanelFeature.State
        /// Every event but the reply's parts as it streams, oldest first.
        /// Setting it works out what the chat shows once, not on every
        /// render or streamed word.
        package var events: IdentifiedArrayOf<SessionEvent> {
            get { log }
            set {
                log = newValue
                var chat = Chat()
                var subagents = SubagentLog()
                var bytes = 0
                for event in newValue {
                    Self.fold(event, into: &chat, &subagents)
                    bytes += Self.bytes(of: event)
                }
                logBytes = bytes
                self.chat = chat
                subagentLog = subagents
                derive()
            }
        }
        private var log: IdentifiedArrayOf<SessionEvent> = []
        /// About how much memory the log holds, counted as its events come,
        /// so nothing reads the whole of it to say. Nothing draws it.
        @ObservationStateIgnored private var logBytes = 0
        /// About how much memory `event` holds: its text, and its id, its
        /// time and its row.
        fileprivate static func bytes(of event: SessionEvent) -> Int { event.kind.bytes + 128 }
        /// The chat, worked out an event at a time: the session's own
        /// agent's events only.
        private var chat = Chat()
        /// The subagents and their chats, worked out from the events that
        /// name the call that started one. Only a subagent's open chat
        /// reads it, so a subagent's step draws nothing else again.
        private var subagentLog = SubagentLog()
        /// The subagents the agent started, in the order they started; a
        /// tool call's row and the Agents list show them.
        package private(set) var subagents: IdentifiedArrayOf<SubagentRun> = []
        /// What the subagents' own steps report used, apart from the
        /// session's `usage`, which holds it only where the backend's own
        /// count leaves it out.
        package private(set) var subagentUsage: Usage?
        /// The subagent whose chat is open to read, by the call that
        /// started it.
        package var shownSubagent: String?
        /// The open subagent's chat, oldest first.
        package var subagentItems: [ChatItem] {
            shownSubagent.map { subagentLog.items(of: $0) } ?? []
        }

        /// Adds `event` to the chat, or, as a subagent's, to that
        /// subagent's own.
        fileprivate static func fold(_ event: SessionEvent, into chat: inout Chat, _ subagents: inout SubagentLog) {
            if let call = event.parentCallID {
                subagents.add(event, under: call)
                return
            }
            chat.add(event)
            if case .turnEnded = event.kind { subagents.turnEnded() }
        }

        /// What the chat shows, oldest first.
        package var items: [ChatItem] { chat.items }
        /// The chat's rows, oldest first: a long reply's parts are each one.
        var lines: [ChatLine] { chat.lines }

        /// A chat's history as it was when the chat was looked away from:
        /// its events and what they were worked out to show, so it shows
        /// again at once and is watched on from its last event.
        package struct History: Equatable, Sendable {
            fileprivate var log: IdentifiedArrayOf<SessionEvent>
            /// About how much memory the log holds.
            fileprivate var logBytes: Int
            fileprivate var chat: Chat
            fileprivate var subagents: SubagentLog
            /// The reply being written, and its parts taken in, so one
            /// still being written goes on from where it was.
            fileprivate var streaming: String
            fileprivate var streamed: Set<String>
            /// The small copies of its sent images read so far, so they
            /// aren't read whole from the daemon again.
            fileprivate var thumbnails: [String: Data]
            /// About how much memory it holds: the text of its events and
            /// its thumbnails.
            package let bytes: Int

            fileprivate init(
                log: IdentifiedArrayOf<SessionEvent>, logBytes: Int, chat: Chat, subagents: SubagentLog,
                streaming: String, streamed: Set<String>, thumbnails: [String: Data]
            ) {
                self.log = log
                self.logBytes = logBytes
                self.chat = chat
                self.subagents = subagents
                self.streaming = streaming
                self.streamed = streamed
                self.thumbnails = thumbnails
                let images = thumbnails.reduce(0) { $0 + $1.key.utf8.count + $1.value.count }
                // The log's part was counted as its events came: counting
                // it here read a long chat whole each time it was left.
                bytes = logBytes + streaming.utf8.count + images
            }
        }

        /// The history to keep when the chat is looked away from; nil
        /// while it hasn't all come or a rewind is replacing it, when what
        /// is here isn't the whole of it.
        package var history: History? {
            guard historyLoaded, !replaying, !rewinding, !log.isEmpty else { return nil }
            return History(
                log: log, logBytes: logBytes, chat: chat, subagents: subagentLog, streaming: streaming,
                streamed: streamed, thumbnails: thumbnails)
        }

        /// Shows `history`, kept when the chat was last looked away from,
        /// without reading it again: watching goes on from its last event.
        package mutating func restore(_ history: History) {
            log = history.log
            logBytes = history.logBytes
            chat = history.chat
            subagentLog = history.subagents
            streaming = history.streaming
            streamed = history.streamed
            thumbnails = history.thumbnails
            derive()
            review.turns = max(review.turns, chat.turnsStarted)
            historyLoaded = true
        }
        /// Whether the history has come since the session opened, so the
        /// chat shows it's loading until then.
        package var historyLoaded = false
        /// How much of a history kept on disk is read, while it's read
        /// before the history comes.
        package var historyLoading: HistoryLoading?
        /// The streamed parts of the reply taken in, so the history sent
        /// again on a reconnect doesn't add them twice. Nothing draws it,
        /// so it's changed in place, not copied and compared each frame.
        @ObservationStateIgnored var streamed: Set<String> = []
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
        /// Claude's all can, so it's known from the start; for the others
        /// it starts as what was last found for the session's model.
        package var readsImages: Bool?
        /// Why the model list couldn't be read, while it still isn't known
        /// whether the model can see images; nil while it's being looked
        /// for, and once it's known.
        package var imagesCheckFailure: String?
        /// What was last found, kept per session while Humata runs, so a
        /// chat opened again offers images at once, or doesn't.
        @Shared var savedReadsImages: ModelReadsImages?
        /// Small copies of the images sent in the chat, by name.
        package var thumbnails: [String: Data] = [:]
        /// Sent images being read for their thumbnails.
        var loadingThumbnails: Set<String> = []
        /// The skills and plugin commands the agent has, for the / list;
        /// nil until first looked for. Kept per session while Humata runs,
        /// so a chat opened again lists them at once, and looks for them
        /// again behind the list.
        @Shared var slashOffered: [SlashChoice]?
        /// Those matching what's typed after a /, best first; empty when
        /// the list isn't showing.
        package var slashChoices: [SlashChoice] = []
        /// The one Return or Tab picks, as arrows move it.
        package var slashSelected = 0
        package var slashLoading = false
        /// Why the skills couldn't be read, while the list shows.
        package var slashFailure: String?
        /// When they were last looked for; they're looked for again when
        /// the list opens after a while, as skills come and go.
        var slashLoadedAt: Date?
        /// The draft the list was closed on with Escape; it stays closed
        /// until the draft changes.
        var slashClosedOn: String?
        /// The name the list shows choices for; nil while it's closed.
        var slashQuery: String?

        /// The chat commands the session's agent takes, as its host last
        /// said; nil until it has. Asked as the chat opens, and kept per
        /// session while Humata runs; a host that can't say leaves what it
        /// said before.
        package var chatCommands: ChatCommands?
        @Shared var savedCommands: ChatCommands?
        /// Whether the host has been asked and the answer has come, even
        /// if it couldn't say: until then a command it may take waits.
        var commandsAnswered = false
        /// The host is being asked which commands it takes.
        var commandsLoading = false
        /// The message Return was pressed on that may be /compact or
        /// /side, held until the host has said whether it takes them.
        var heldSend: HeldSend?
        /// Whether a message is held so, which the box shows.
        package var holdingSend: Bool { heldSend != nil }
        /// The pastes that went as a /side question's first message, taken
        /// out of the box once its side chat is made.
        var sidePastes: [Paste.ID] = []
        /// Where the chat can be rewound to, to pick from, after /rewind;
        /// nil while closed. Only the box reads it.
        package var rewindPicker: RewindPicker?
        /// Asks whether to stop the running turn to compact.
        @Presents package var compactDialog: ConfirmationDialogState<Action.CompactChoice>?
        /// A compaction was asked for and its turn hasn't started yet.
        package var compactStarting = false

        /// Whether a / and a name are being typed, with the list not
        /// closed: the list, or why it's empty, shows. Only for the draft
        /// the list was made for, so text put in the box another way
        /// doesn't show an old list.
        package var slashOpen: Bool { slashQuery != nil && slashQuery == SlashChoice.query(in: draft) }
        /// The thinking the person opened, by item id.
        package var expandedThinking: Set<String> = []
        /// The tool calls the person opened, by call id.
        package var expandedTools: Set<String> = []
        /// Whether the daemon stopped sending events; watching again resumes.
        package var disconnected = false
        /// Why watching ended, when it's more than the connection dropping.
        package var watchFailure: String?
        /// Times in a row watching has ended without a new event, which spaces
        /// out the next try.
        package var watchRetries = 0
        /// The person's message being edited, by event id; nil when none is.
        package var editing: String?
        /// Its editor is to take the caret, once: set as the edit starts
        /// and cleared when the editor has, so a row scrolled back into
        /// view doesn't take the caret from the box.
        package var editCaret = false
        /// The edited message's text.
        package var edited = ""
        /// The names of the edited message's images, which go to the fork
        /// with it.
        package var editedImages: [String] = []
        /// The queued message being edited in place, by queue id.
        package var editingQueued: String?
        /// Its text as edited so far.
        package var queuedEdit = ""
        /// The fork made for an edited message that didn't reach it: it
        /// opens once the alert that says so is dismissed.
        package var unsentFork: Session?
        @Presents package var alert: AlertState<Never>?
        /// Changing the chat's model from the composer's pill.
        package var modelSwitch = ModelSwitch()
        /// The daemon is being asked to switch plan mode.
        package var planSwitching = false
        /// Asks how far to rewind: the chat, or the chat and its files.
        @Presents package var rewindDialog: ConfirmationDialogState<Action.RewindChoice>?
        /// A rewind or its undo is under way.
        package var rewinding = false
        /// The message the last rewind put back in the box, so undoing it
        /// takes it out again if it wasn't changed.
        var rewoundText: String?
        /// The chip it went back as, when it was too long for the field.
        var rewoundPaste: Paste.ID?
        /// The images the last rewind put back in the box with it.
        var rewoundImages: [DraftImage.ID] = []
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
        /// A fork, an edited message's fork or a new chat under this one
        /// is being made.
        package var forking = false
        /// The kind of chat being made under this one, while one is, so
        /// what's shown over the view says which.
        package var makingChild: ChildChat?
        /// Whether its Approve, Deny and Steer Now take their keyboard
        /// shortcuts: only the column in use does, when two sessions show
        /// side by side. Whoever shows the session sets it.
        package var takesShortcuts = true
        /// Whether Steer Now takes ⌘Return: not while an earlier message
        /// is edited, when the key is its editor's.
        package var steerTakesShortcut: Bool { takesShortcuts && editing == nil }
        /// Shown beside another session, as a side chat is: its chat
        /// alone, under whatever bar its column gives it.
        package var beside = false
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
        /// The request was answered and the daemon hasn't said so yet, so
        /// the strip's buttons can't answer it a second time. Whoever
        /// shows the session clears it when the answer lands or fails.
        package var answeringRequest = false
        /// How the strip answered it, which it says while that's under
        /// way; nil when it was answered elsewhere, such as in the Inbox.
        package var requestAnswer: RequestAnswering?
        /// Added to by whoever wants the box to take the caret, such as
        /// after a picker over it that had the caret has closed.
        package var boxFocus = 0
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
            planPanel = PlanPanelFeature.State(projectID: session.projectID, itemID: session.itemID)
            let saved = Shared(wrappedValue: "", .appStorage(Self.draftKey(session.id)))
            _savedDraft = saved
            draft = saved.wrappedValue
            let pasted = Shared(wrappedValue: "", .appStorage(Self.draftKey(session.id) + "_pastes"))
            _savedPastes = pasted
            pastes = Paste.decode(pasted.wrappedValue)
            _slashOffered = Shared(wrappedValue: nil, .inMemory("slash_" + session.id))
            let commands = Shared<ChatCommands?>(wrappedValue: nil, .inMemory("commands_" + session.id))
            _savedCommands = commands
            chatCommands = commands.wrappedValue
            commandsAnswered = commands.wrappedValue != nil
            let images = Shared<ModelReadsImages?>(wrappedValue: nil, .inMemory("reads_images_" + session.id))
            _savedReadsImages = images
            switch session.backend {
            case .claude, .scripted: readsImages = true
            case .copilot, .local:
                // Only for the model it was found for.
                readsImages = images.wrappedValue.flatMap { $0.model == session.model ? $0.reads : nil }
            }
        }

        /// Whether there's anything to send.
        package var canSend: Bool {
            guard !images.contains(where: \.isPreparing) else { return false }
            // Images wait until it's known the model can see them.
            guard images.isEmpty || readsImages == true else { return false }
            return !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !pastes.isEmpty
                || !images.isEmpty
        }

        /// Whether the box is on screen, as the view lays it out: under the
        /// chat, and beside a file or a spec that has the chat next to it.
        var showsBox: Bool {
            switch tab {
            case .chat: true
            case .files: docs.focused
            case .spec: specs.focused != nil
            case .review, .terminal: false
            }
        }

        /// Whether the model is known to see images, so the box offers to
        /// add them.
        package var takesImages: Bool { readsImages == true && !session.archived }

        /// Whether there's a turn to stop that hasn't been told to yet.
        package var canStop: Bool { isWorking && !stopping && !session.archived }

        /// The chat row of the highlighted event, once the history has it:
        /// a tool's row for its call or output, otherwise the event's own.
        package var highlightedItem: String? {
            guard let highlighted, let index = events.index(id: highlighted) else { return nil }
            let event = events[index]
            switch event.kind {
            case .toolCompleted(let callID, _, _): return row(ofCall: callID, before: index + 1)
            default: return event.id
            }
        }

        /// The chat row of the agent's tool call `callID`: the event that
        /// started it, the latest one before `end`, as an agent can use one
        /// id again in a later turn.
        func row(ofCall callID: String, before end: Int? = nil) -> String? {
            let end = end ?? events.count
            let started = events.elements[..<end].last {
                if case .toolStarted(callID, _, _) = $0.kind { true } else { false }
            }
            return started?.id
        }

        /// The chat row of the proposal not decided yet that comes first in
        /// the chat, whatever order they're listed in; nil when none of them
        /// has a row here.
        var firstWaitingProposalRow: String? {
            pendingProposals
                .compactMap { proposal in row(ofCall: proposal.id).flatMap { events.index(id: $0) } }
                .min()
                .map { events[$0].id }
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
            guard !beside else { return [.chat] }
            var tabs: [Tab] = session.projectID == nil ? [.chat, .files] : [.chat, .review, .files, .spec]
            if !session.archived { tabs.append(.terminal) }
            return tabs
        }

        /// Whether it has code of a project's to land or update: not a
        /// planning session, and not archived.
        package var ships: Bool {
            session.projectID != nil && !session.planning && !session.archived && !session.readOnly
        }

        /// Whether a turn has started and not yet ended. Kept, as views
        /// read it on every key typed.
        package private(set) var isWorking = false
        /// Whether the turn under way sums up the conversation: it has no
        /// message of the person's to stop and edit.
        package private(set) var isCompacting = false

        /// Takes from the chat what its events add up to, kept there as each
        /// event comes, so views read it instead of working it out again.
        /// Sets only what changed, so views showing the rest aren't drawn
        /// again.
        private mutating func derive() {
            if isWorking != chat.working { isWorking = chat.working }
            let working = isWorking
            let compacting = working && chat.compacting
            if isCompacting != compacting { isCompacting = compacting }
            if stopping, !working { stopping = false }
            if let land, land.turnRunning != working { self.land?.turnRunning = working }
            if lastMessage != chat.lastMessage { lastMessage = chat.lastMessage }
            if lastMessageOwn != chat.lastMessageOwn { lastMessageOwn = chat.lastMessageOwn }
            if canUndoRewind != chat.canUndoRewind { canUndoRewind = chat.canUndoRewind }
            if lastImages != chat.lastImages { lastImages = chat.lastImages }
            if lastTurnFailed != chat.lastTurnFailed { lastTurnFailed = chat.lastTurnFailed }
            if !lastTurnFailed, retrying { retrying = false }
            if subagents != subagentLog.runs { subagents = subagentLog.runs }
            if subagentUsage != subagentLog.usage { subagentUsage = subagentLog.usage }
            // A rewind can take the open subagent out of the history.
            if let shown = shownSubagent, subagents[id: shown] == nil { shownSubagent = nil }
            let usage = totalUsage
            if self.usage != usage { self.usage = usage }
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
            // A compaction that failed has no message to send again.
            let retryable = lastTurnFailed && lastMessage != nil && !chat.compacting ? chat.lastFailure : nil
            if retryableFailure != retryable { retryableFailure = retryable }
        }

        /// What the session used, its subagents too. Every backend counts
        /// a turn's main agent apart from its subagents (Claude Code's
        /// result.usage covers its main loop only), so theirs is added.
        private var totalUsage: Usage? {
            guard let subagents = subagentLog.usage else { return chat.usage }
            return chat.usage?.adding(subagents) ?? subagents
        }

        /// Whether the agent is waiting on the person to answer a request.
        package var isWaiting: Bool { !pendingPermissions.isEmpty }

        /// The proposals not decided yet, oldest first.
        package var pendingProposals: [Proposal] {
            proposals.proposals.filter { $0.state == .pending }
        }

        /// The last message sent to the session, to send again after a
        /// failed turn.
        package private(set) var lastMessage: String?
        /// Whether it was the person's own, to stop and edit: not another
        /// session's agent's or a notice of Humata's.
        package private(set) var lastMessageOwn = false
        /// Whether the chat's last rewind can still be undone: until a
        /// message is sent after it, or it's undone.
        package private(set) var canUndoRewind = false
        /// The names of the images sent with it, which go with it again.
        package private(set) var lastImages: [String] = []

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
            var editedImages: [String]
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
                    editedImages: editedImages, editingQueued: editingQueued, queuedEdit: queuedEdit,
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
                editedImages = newValue.editedImages
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

        /// Whether `folded`, a history worked out ahead, is all the chat
        /// will hold once it's taken in: the chat has nothing yet, and the
        /// history says no event twice. It's then taken as it is, without
        /// reading its events again.
        func opens(with folded: FoldedHistory) -> Bool {
            folded.unique && folded.count > 0 && log.isEmpty && streaming.isEmpty && streamed.isEmpty
        }

        /// Takes in `batch` and works out the chat once, so a history of
        /// thousands of events shows at once instead of one at a time.
        /// `folded` is the chat already worked out for the batch, off the
        /// main thread: it's taken as it is when the batch is the whole
        /// history of a chat that had none, each event once.
        mutating func receive(_ batch: [SessionEvent], folded: FoldedHistory? = nil) {
            if let folded, opens(with: folded) {
                // Everything the batch comes to was worked out ahead, its
                // log too: nothing here reads the batch.
                if folded.changedCode { review.stale = true }
                if folded.turnEnded, stopping { stopping = false }
                if self.streaming != folded.streaming { self.streaming = folded.streaming }
                self.streamed = folded.streamed
                self.log = folded.log
                self.logBytes = folded.bytes
                self.chat = folded.chat
                self.subagentLog = folded.subagents
                derive()
                if folded.turnStarted { review.turns = max(review.turns, self.chat.turnsStarted) }
                return
            }
            let opensEmpty = log.isEmpty
            // A reply's words alone, as most batches are while it streams,
            // change only the reply being written: the log stays where it
            // is, so nothing that reads it is drawn again for them.
            let wordsOnly = batch.allSatisfy { if case .assistantDelta = $0.kind { true } else { false } }
            if wordsOnly {
                var streaming = self.streaming
                // A subagent's words aren't the reply's: its whole message
                // comes after them, into its own chat.
                for event in batch where event.parentCallID == nil {
                    if case .assistantDelta(let text) = event.kind, streamed.insert(event.id).inserted {
                        streaming += text
                    }
                }
                if streaming != self.streaming { self.streaming = streaming }
                return
            }
            // The log is taken out while it's changed and put back once: the
            // state is observed, and changing it where it is copies it whole
            // for each event, which for a long history is seconds.
            var log = self.log
            self.log = []
            var added: [SessionEvent] = []
            var addedBytes = 0
            var replaced = false
            var stale = false
            var turnEnded = false
            var streaming = self.streaming
            for event in batch {
                switch event.kind {
                case .assistantDelta(let text):
                    if event.parentCallID == nil, self.streamed.insert(event.id).inserted { streaming += text }
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
                    addedBytes += Self.bytes(of: event)
                    // Only a reply that's new ends the one being written: a
                    // history sent again holds the replies before it.
                    // A subagent's message leaves the agent's own as it is.
                    switch event.kind {
                    case .assistantMessage where event.parentCallID == nil, .turnEnded:
                        streaming = ""
                        self.streamed = []
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
            guard replaced || !added.isEmpty else {
                self.log = log
                return
            }
            if replaced {
                events = log
            } else {
                self.log = log
                logBytes += addedBytes
                // Live events go on the end of the chat; only a changed
                // event means working it out again. Taken out to change, as
                // the log is.
                if let folded, opensEmpty, folded.count == added.count {
                    // Every event of the batch was new to an empty chat, so
                    // what was worked out for the batch ahead is the chat.
                    self.chat = folded.chat
                    self.subagentLog = folded.subagents
                    derive()
                    if batch.contains(where: { $0.kind == .turnStarted }) {
                        review.turns = max(review.turns, self.chat.turnsStarted)
                    }
                    return
                }
                // The chat is set only for an event of its own, so a
                // subagent's steps leave its rows as they are.
                var chat = self.chat
                var subagents = self.subagentLog
                let ownEvents = added.contains { $0.parentCallID == nil }
                let subagentEvents = added.contains { event in
                    if event.parentCallID != nil { return true }
                    if case .turnEnded = event.kind { return !subagents.runs.isEmpty }
                    return false
                }
                if ownEvents { self.chat = Chat() }
                if subagentEvents { self.subagentLog = SubagentLog() }
                for event in added { Self.fold(event, into: &chat, &subagents) }
                if ownEvents { self.chat = chat }
                if subagentEvents { self.subagentLog = subagents }
                derive()
            }
            if batch.contains(where: { $0.kind == .turnStarted }) {
                review.turns = max(review.turns, chat.turnsStarted)
            }
        }
    }

    /// The /rewind picker: where the chat can be cut back to, narrowed by
    /// what's typed in its own field.
    package struct RewindPicker: Equatable, Sendable {
        package var query: String
        /// Every point, newest first.
        var all: [RewindPoint]
        /// Those matching the query, best first.
        package var points: [RewindPoint]
        /// The one Return picks, as arrows move it.
        package var selected = 0

        init(query: String, all: [RewindPoint]) {
            self.query = query
            self.all = all
            points = RewindPoint.matching(query, in: all)
        }
    }

    /// A message Return was pressed on, held as it was typed.
    struct HeldSend: Equatable, Sendable {
        var draft: String
        var mode: SendMode
    }

    /// What came of asking a session's host which chat commands its agent
    /// takes.
    package enum CommandsAsked: Equatable, Sendable {
        /// The host said which.
        case said(ChatCommands)
        /// The host refused the question, as one from before chat commands
        /// does: its agent takes none.
        case takesNone
        /// The host couldn't be asked, so it hasn't answered.
        case unanswered

        /// What the host said, when it did.
        var commands: ChatCommands? {
            if case .said(let commands) = self { commands } else { nil }
        }
    }

    /// How the strip over the box answered the agent's request.
    package enum RequestAnswering: Equatable, Sendable {
        case allowing
        case declining
        case moving
        /// A move is being put off: dismissed, with the task left where
        /// it is.
        case puttingOff
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
        /// starts; `folded` is the chat worked out for them ahead, when
        /// they are the history of a chat that opened empty.
        case receivedAll([SessionEvent], folded: FoldedHistory? = nil)
        /// The history a watch starts with has all come; what comes after
        /// is new.
        case historyReplayed
        /// The history is read from disk first: `read` bytes of `total`.
        case historyLoading(read: UInt64, total: UInt64)
        case watchEnded(failure: String?)
        case sendTapped(SendMode)
        /// The skills were looked for, and the agent's host asked which
        /// chat commands it takes.
        case slashLoaded(Result<[SlashChoice], SlashLoadFailure>, commands: CommandsAsked)
        /// The agent's host was asked which chat commands it takes, as
        /// the chat opened or for a message held.
        case commandsLoaded(CommandsAsked)
        /// An arrow moved the / list's pick by `by` rows.
        case slashMoved(by: Int)
        case slashPicked(String)
        /// Tab in the / list: writes the highlighted choice's name in the
        /// box, to type what follows it; nothing is carried out.
        case slashCompleted
        /// Escape closed the / list.
        case slashClosed
        /// What's typed in the /rewind picker's field.
        case rewindPickerQueryChanged(String)
        /// An arrow moved the /rewind picker's pick by `by` rows.
        case rewindPickerMoved(by: Int)
        /// Picked the event to rewind to; asks how far, as its row does.
        case rewindPickerPicked(String)
        case rewindPickerClosed
        /// Opens the /rewind picker from outside the box, such as from a
        /// menu; what's typed in the box stays.
        case rewindPickerAsked
        /// Has the agent sum up the conversation, keeping `focus` in mind;
        /// asks first when that stops a turn.
        case compactAsked(focus: String)
        case compactDialog(PresentationAction<CompactChoice>)
        /// The compaction's turn started; the chat shows how it goes.
        case compactStarted
        /// It didn't start; a `focus` typed goes back in the box.
        case compactFailed(focus: String, message: String)
        /// A message sent from the box reached the daemon.
        case sent
        /// The message wasn't sent; what was typed, pasted and added goes
        /// back in the box, before anything put there since.
        case sendFailed(text: String, pastes: [Paste], images: [DraftImage], message: String)
        /// The session's model can, or can't, see images.
        case readsImagesFound(Bool)
        /// Whether it can see them couldn’t be found out, with why.
        case readsImagesUnknown(String)
        /// Finds out again, after it couldn't be.
        case readsImagesAsked
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
        /// A pasted chip's text was changed; with nothing left, it's taken
        /// out.
        case pasteEdited(id: String, text: String)
        /// A pasted chip's text, as last changed, goes into the box after
        /// what's typed, to edit there, if it's short enough for the field.
        case pasteMovedToBox(id: String, text: String)
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
        /// Puts the move the agent asked for off: takes the request away
        /// and leaves the task where it is.
        case movePutOff
        /// Allows the session the agent asked to start or the message it
        /// asked to send, from the strip over the box.
        case requestAllowed
        /// Declines it.
        case requestDeclined
        /// Scrolls the chat to the first proposal waiting to be decided.
        case showProposalsTapped
        /// The chat scrolled to the target.
        case scrolled
        /// Asks how far to rewind to before `event`: one of the person's
        /// messages, or a compaction.
        case rewindTapped(String)
        case rewindDialog(PresentationAction<RewindChoice>)
        /// The chat was cut back; `text` goes back in the box, unless it's
        /// empty, as for a compaction.
        case rewound(String)
        /// The images of a sent message, read again to go back in the box
        /// with its text: after a rewind, or for Stop and Edit.
        case sentImagesRead([DraftImage], rewound: Bool)
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
        /// The composer's model pill and its list: the model changes from
        /// the next turn.
        case model(ModelSwitchAction)
        /// The composer's Plan toggle: plan mode on or off from the next
        /// turn.
        case plan(PlanSwitchAction)
        case thinkingToggled(id: String)
        case toolToggled(id: String)
        /// Opens the chat of the subagent the tool call `callID` started,
        /// to read.
        case subagentTapped(callID: String)
        case subagentClosed
        /// Forks the session before `event`, or with all of it if nil. A
        /// chat-only fork keeps working in this session's copy of the code.
        case forkTapped(before: String?, chatOnly: Bool)
        /// Forks with everything up to the end of the highlighted event's
        /// turn, with the code as it was then.
        case forkFromHighlightTapped
        case highlightDismissed
        /// Starts editing the person's message `event`.
        case editTapped(String)
        /// The message's editor took the caret.
        case editCaretTaken
        case editCancelled
        /// Forks before the edited message, with the code as it was, and
        /// sends the fork the edited text with the message's images.
        case editSent
        case forked(Session)
        /// The fork for an edited message was made, but the message
        /// couldn't be sent to it, for the reason `message` gives: this
        /// session says so, then the fork opens with the text in its box.
        /// `images` is how many images the message had, which didn't go.
        case forkSendFailed(fork: Session, text: String, images: Int, message: String)
        /// Starts a chat as this session's child: an empty one, which
        /// opens in its place, or a read-only side chat, which opens
        /// beside it and is sent `message` unless that's empty.
        case childTapped(ChildChat, message: String)
        /// The side chat was made. `unsent` is its first message if that
        /// couldn't be sent, with `failure` saying why: it waits in the
        /// side chat's box.
        case sideMade(Session, unsent: String, failure: String = "")
        case failed(Doing, String)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)
        case tabPicked(Tab)
        case review(ReviewFeature.Action)
        case docs(DocsFeature.Action)
        case specs(SpecsFeature.Action)
        case planPanel(PlanPanelFeature.Action)
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
            case startingChild
            case updating
            case answering(requestID: String)
        }

        package enum RewindChoice: Equatable {
            /// Cut the chat back to before `event`, leaving the code.
            case chat(String)
            /// Put the code back as it was then, too.
            case chatAndFiles(String)
        }

        package enum CompactChoice: Equatable {
            /// Stop the running turn, then compact with `focus` in mind.
            case stopAndCompact(focus: String)
        }

        @CasePathable
        package enum Delegate: Equatable {
            /// A fork was made, or work started on a proposed task; open it.
            case openSession(Session)
            /// A side chat was made; open it beside this session.
            case openSide(Session)
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
    @Dependency(\.date) var date
    private enum CancelID { case watch, seen, activations, slash }

    /// Why the / list couldn't be filled.
    package struct SlashLoadFailure: Error, Equatable {
        package var message: String
    }

    /// Shows the / list for what's typed, and looks for the skills when it
    /// opens for the first time, after a while, or again after looking
    /// failed.
    private func slashFollows(_ state: inout State) -> Effect<Action> {
        // Escape holds only for the draft it was pressed on.
        if state.slashClosedOn != state.draft { state.slashClosedOn = nil }
        guard let query = SlashChoice.query(in: state.draft), state.slashClosedOn == nil else {
            slashHidden(&state)
            return .none
        }
        if state.rewindPicker != nil { state.rewindPicker = nil }
        let opening = state.slashQuery == nil
        // Humata's own commands show before the skills are first found.
        let offered =
            state.slashOffered ?? SlashChoice.builtIns(for: state.session, commands: state.chatCommands)
        let choices = SlashChoice.matching(query, in: offered)
        // The highlight stays only on the same list; a new one starts at
        // its best match.
        if opening || choices != state.slashChoices { state.slashSelected = 0 }
        state.slashQuery = query
        state.slashChoices = choices
        let stale = state.slashLoadedAt.map { date.now.timeIntervalSince($0) > Self.slashFresh } ?? true
        let retry = opening && state.slashFailure != nil
        guard stale || retry, !state.slashLoading else { return .none }
        state.slashLoading = true
        let session = state.session
        let known = state.chatCommands
        return .run { send in
            // A host from before chat commands can't say which it takes:
            // it's offered none of them, and its skills all the same. One
            // that couldn't be asked this time is offered what it last said.
            async let asked = Self.commandsAsked(of: session.id, daemon)
            do {
                async let skills = daemon.skills()
                async let plugins = daemon.plugins()
                async let projects = daemon.projects()
                let commands = await asked
                let project = try await projects.first { $0.id == session.projectID }
                let offered = SlashChoice.offered(
                    to: session, skills: try await skills, plugins: try await plugins, project: project,
                    commands: commands.commands ?? known)
                await send(.slashLoaded(.success(offered), commands: commands))
            } catch {
                let failure = SlashLoadFailure(
                    message: "Couldn't look for skills: \(error.localizedDescription) "
                        + "Check the daemon and hosts are reachable, then type / again.")
                await send(.slashLoaded(.failure(failure), commands: await asked))
            }
        }
        .cancellable(id: CancelID.slash, cancelInFlight: true)
    }

    /// How long skills found stay fresh before the list looks again.
    static let slashFresh: TimeInterval = 30

    /// Takes the / list away, as when the draft is changed other than by
    /// typing; typing again shows it if the draft still asks for it.
    private func slashHidden(_ state: inout State) {
        state.slashQuery = nil
        state.slashChoices = []
        state.slashSelected = 0
        // Text in the box closes the /rewind picker; the box echoing back
        // the empty draft the picker left it with doesn't.
        if !state.draft.isEmpty, state.rewindPicker != nil { state.rewindPicker = nil }
    }

    /// Whether the session is offered `command`, as the / list would list
    /// it.
    private static func offers(_ command: SlashChoice.BuiltIn, _ state: State) -> Bool {
        SlashChoice.builtIns(for: state.session, commands: state.chatCommands).contains { $0.builtIn == command }
    }

    /// Asks the session's host which chat commands its agent takes, unless
    /// it's being asked already, by this or by the / list.
    private func askCommands(_ state: inout State) -> Effect<Action> {
        guard !state.commandsLoading, !state.slashLoading else { return .none }
        state.commandsLoading = true
        let session = state.session.id
        return .run { [daemon] send in
            await send(.commandsLoaded(await Self.commandsAsked(of: session, daemon)))
        }
    }

    /// Asks `session`'s host which chat commands its agent takes. A host
    /// that refuses the question takes none; one that couldn't be reached
    /// hasn't answered.
    private static func commandsAsked(of session: Session.ID, _ daemon: DaemonClient) async -> CommandsAsked {
        do {
            return .said(try await daemon.chatCommands(session))
        } catch let error as DaemonError {
            // Refused for the host being away is this Mac's daemon saying
            // it couldn't ask, not the host saying no.
            if case .refused(.hostUnavailable, _) = error { return .unanswered }
            if case .refused = error { return .takesNone }
            return .unanswered
        } catch {
            return .unanswered
        }
    }

    /// Takes in what came of asking the host for its chat commands. One
    /// that takes none leaves what it said before, and has answered: a
    /// message held for the answer is sent now, unless the box has changed
    /// since. One that couldn't be asked hasn't: the message held stays in
    /// the box, with why.
    private func commandsAnswered(_ asked: CommandsAsked, _ state: inout State) -> Effect<Action> {
        switch asked {
        case .said(let commands):
            if !state.commandsAnswered { state.commandsAnswered = true }
            if state.chatCommands != commands {
                state.chatCommands = commands
                state.$savedCommands.withLock { $0 = commands }
            }
        case .takesNone:
            if !state.commandsAnswered { state.commandsAnswered = true }
        case .unanswered:
            guard state.heldSend != nil else { return .none }
            state.heldSend = nil
            state.alert = Self.alert(
                "The session’s host didn’t answer",
                "Humata couldn’t ask the host whether its agent takes this command, so nothing was sent and the "
                    + "command is still in the box. Press Return to try again, or check the host in Hosts.")
            return .none
        }
        guard let held = state.heldSend else { return .none }
        state.heldSend = nil
        return held.draft == state.draft ? .send(.sendTapped(held.mode)) : .none
    }

    private func clearDraft(_ state: inout State) {
        state.draft = ""
        state.$savedDraft.withLock { $0 = "" }
        slashHidden(&state)
    }

    /// Opens the /rewind picker on what `query` finds: in place of the
    /// command typed in the box, or with `typed` false, as from a menu,
    /// leaving the box as it is.
    private func rewindPickerOpened(_ query: String, typed: Bool = true, _ state: inout State) {
        // The chat isn't all here yet, so neither is what it can be cut
        // back to: the command stays in the box for when it is.
        guard state.historyLoaded else {
            state.alert = Self.alert(
                "The chat is still opening",
                "What it can be rewound to isn't known until the chat shows. "
                    + (typed
                        ? "The command is still in the box: press Return again once the chat is here."
                        : "Choose Rewind again once the chat is here."))
            return
        }
        if typed { clearDraft(&state) }
        let points = RewindPoint.points(in: state.events)
        guard !points.isEmpty else {
            state.alert = Self.alert(
                "Nothing to rewind to yet",
                "A chat is rewound to before a message you sent, or before a compaction, and this one has "
                    + "neither. Send a message first.")
            return
        }
        state.rewindPicker = RewindPicker(query: query, all: points)
    }

    /// Asks for the compaction, taking the command out of the box.
    private func compact(_ focus: String, stop: Bool, _ state: inout State) -> Effect<Action> {
        if SlashChoice.command(in: state.draft)?.command == .compact { clearDraft(&state) }
        state.compactStarting = true
        let session = state.session.id
        return .run { send in
            try await daemon.compact(session, focus, stop)
            await send(.compactStarted)
        } catch: { error, send in
            await send(.compactFailed(focus: focus, message: error.localizedDescription))
        }
    }

    private func received(
        _ events: [SessionEvent], folded: FoldedHistory? = nil, _ state: inout State
    ) -> Effect<Action> {
        // A watch sends the history first. Only something new shows the
        // session is live: an offline host's cached history ends its watch
        // at once, and must not be fetched every second.
        // A history worked out ahead for a chat that has nothing yet says
        // all of this itself, so its events aren't read again here, on the
        // main thread.
        let whole = folded.flatMap { state.opens(with: $0) ? $0 : nil }
        let new = whole == nil ? events.filter { state.events[id: $0.id] == nil } : []
        if whole != nil || new.contains(where: \.kind.isLive) { state.watchRetries = 0 }
        // A rewind or its undo replaced the history: read it again from
        // the start, once for each. One in the history of a chat that
        // opened empty happened before it was read, so that history
        // already is what it left.
        let restart: String?
        if let whole {
            restart = whole.restart
        } else {
            restart =
                new.last(where: { event in
                    switch event.kind {
                    case .rewound, .rewindUndone: true
                    default: false
                    }
                })?.id
        }
        if let restart, restart != state.restartedFor {
            state.restartedFor = restart
            if !state.replaying, !state.events.isEmpty {
                state.events = []
                state.streaming = ""
                state.streamed = []
                return .send(.task)
            }
        }
        let editedBefore = state.editingQueued.flatMap { id in state.queued.first { $0.id == id }?.text }
        state.receive(events, folded: folded)
        // A history still being sent again isn't yet what the queue holds.
        if !state.replaying { queuedEditLeftBehind(was: editedBefore, &state) }
        var effects: [Effect<Action>] = []
        let proposed =
            whole?.proposed
            ?? new.contains(where: { event in
                if case .toolCompleted(let call, true, _) = event.kind { state.proposes(call) } else { false }
            })
        if proposed { effects.append(.send(.proposals(.refresh))) }
        // The agent may have answered the review, or changed the code or
        // the files.
        let turnEnded =
            whole?.turnEnded ?? new.contains(where: { if case .turnEnded = $0.kind { true } else { false } })
        if turnEnded {
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

    /// The images `names` of a message sent in `session`, read again as
    /// they were sent, ready for the box. One that can't be read is left
    /// out: the text goes back all the same.
    private static func sentImages(
        _ names: [String], of session: Session.ID, _ daemon: DaemonClient, _ uuid: UUIDGenerator
    ) async -> [DraftImage] {
        var images: [DraftImage] = []
        for name in names {
            guard let image = try? await daemon.readImage(session, name) else { continue }
            let thumbnail = ImagePrep.thumbnail(image.data, side: ImagePrep.thumbnailSide)
            images.append(DraftImage(id: uuid().uuidString, name: name, image: image, thumbnail: thumbnail))
        }
        return images
    }

    /// Closes the edit of a queued message that's no longer queued, as
    /// once its turn started: it ran as it was, so an edit that changed it
    /// goes in the box, and the person is told. `original` is its text
    /// while it was queued, when that's known.
    private func queuedEditLeftBehind(was original: String?, _ state: inout State) {
        guard state.historyLoaded, let id = state.editingQueued,
            !state.queued.contains(where: { $0.id == id })
        else { return }
        let text = state.queuedEdit.trimmingCharacters(in: .whitespacesAndNewlines)
        state.editingQueued = nil
        state.queuedEdit = ""
        guard !text.isEmpty, text != original?.trimmingCharacters(in: .whitespacesAndNewlines) else { return }
        putInBox(text, ahead: false, &state)
        state.alert = AlertState {
            TextState("Its turn already started")
        } message: {
            TextState(
                """
                The message left the queue while you were editing it, so it ran as it was. Your edit is in \
                the box: send it to follow up, or steer the running turn with it.
                """)
        }
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
        let session = state.session
        switch session.backend {
        case .claude, .scripted:
            if state.readsImages == nil { state.readsImages = true }
            return .none
        case .copilot, .local:
            // Asked each time the chat opens, behind what was last found:
            // the model, or what it can do, may have changed.
            return .run { send in
                // A list that couldn't be read says nothing of the model:
                // what was last found stands, and images wait if nothing
                // was.
                let models: [ModelChoice]
                do {
                    models = try await daemon.models(session.host, session.backend, nil)
                } catch {
                    await send(.readsImagesUnknown(error.localizedDescription))
                    return
                }
                let choice = models.first { $0.id == session.model } ?? (session.model == nil ? models.first : nil)
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
        Scope(state: \.planPanel, action: \.planPanel) {
            PlanPanelFeature()
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
                // A message held for the host's answer was changed: it
                // waits for Return again.
                if let held = state.heldSend, held.draft != state.draft { state.heldSend = nil }
                return slashFollows(&state)

            case .commandsLoaded(let commands):
                state.commandsLoading = false
                let held = commandsAnswered(commands, &state)
                // Refreshes a list showing, which may list more now.
                guard state.slashOpen else { return held }
                return .merge(held, slashFollows(&state))

            case .slashLoaded(let result, let commands):
                state.slashLoading = false
                state.slashLoadedAt = date.now
                let held = commandsAnswered(commands, &state)
                switch result {
                case .success(let offered):
                    if state.slashOffered != offered { state.$slashOffered.withLock { $0 = offered } }
                    state.slashFailure = nil
                case .failure(let failure):
                    state.slashFailure = failure.message
                }
                // Refreshes a list showing; one closed since stays closed.
                guard state.slashOpen else { return held }
                return .merge(held, slashFollows(&state))

            case .slashMoved(let by):
                guard !state.slashChoices.isEmpty else { return .none }
                let count = state.slashChoices.count
                state.slashSelected = ((state.slashSelected + by) % count + count) % count
                return .none

            case .slashPicked(let name):
                switch state.slashChoices.first(where: { $0.id == name })?.builtIn {
                case .rewind?:
                    rewindPickerOpened("", &state)
                    return .none
                case .compact? where state.chatCommands?.compactTakesFocus == false:
                    // Nothing more to type: it's asked for at once.
                    clearDraft(&state)
                    return .send(.compactAsked(focus: ""))
                case .newHere?, .fork?, .side?:
                    // Typed in full, it's carried out; picked from part of
                    // its name, what may follow it is typed first.
                    guard SlashChoice.query(in: state.draft) == name else { break }
                    state.draft = "/\(name) "
                    slashHidden(&state)
                    return .send(.sendTapped(.enqueue))
                case .compact?, nil:
                    break
                }
                state.draft = "/\(name) "
                state.$savedDraft.withLock { $0 = state.draft }
                slashHidden(&state)
                return .none

            case .slashCompleted:
                guard state.slashOpen, state.slashChoices.indices.contains(state.slashSelected) else {
                    return .none
                }
                state.draft = "/\(state.slashChoices[state.slashSelected].id) "
                state.$savedDraft.withLock { $0 = state.draft }
                slashHidden(&state)
                return .none

            case .rewindPickerQueryChanged(let query):
                guard var picker = state.rewindPicker, picker.query != query else { return .none }
                picker.query = query
                picker.points = RewindPoint.matching(query, in: picker.all)
                picker.selected = 0
                state.rewindPicker = picker
                return .none

            case .rewindPickerMoved(let by):
                guard let count = state.rewindPicker?.points.count, count > 0 else { return .none }
                let selected = state.rewindPicker?.selected ?? 0
                state.rewindPicker?.selected = ((selected + by) % count + count) % count
                return .none

            case .rewindPickerPicked(let id):
                guard state.rewindPicker != nil else { return .none }
                state.rewindPicker = nil
                // Its field had the caret, and is gone: the box has it
                // after, whatever comes of the rewind.
                state.boxFocus += 1
                return .send(.rewindTapped(id))

            case .rewindPickerClosed:
                state.rewindPicker = nil
                // Its field had the caret, and is gone.
                state.boxFocus += 1
                return .none

            case .rewindPickerAsked:
                guard !state.session.archived, !state.rewinding else { return .none }
                // A /rewind typed and not yet sent goes, as Return on it
                // would take it; anything else in the box stays.
                let typed = SlashChoice.command(in: state.draft)?.command == .rewind
                rewindPickerOpened("", typed: typed, &state)
                // Only the box shows the picker, so a view without the box
                // gives way to the chat.
                guard state.rewindPicker != nil, !state.showsBox else { return .none }
                return .send(.tabPicked(.chat))

            case .compactAsked(let focus):
                guard !state.compactStarting, !state.session.archived else { return .none }
                guard state.isWorking else { return compact(focus, stop: false, &state) }
                state.compactDialog = ConfirmationDialogState {
                    TextState("Stop the turn and compact?")
                } actions: {
                    ButtonState(action: .stopAndCompact(focus: focus)) { TextState("Stop and Compact") }
                    ButtonState(role: .cancel) { TextState("Cancel") }
                } message: {
                    TextState(
                        "The agent’s turn stops where it is, and the agent sums up the conversation so far. "
                            + "Queued messages run after. Or cancel, and compact once the turn has ended.")
                }
                return .none

            case .compactDialog(.presented(.stopAndCompact(let focus))):
                guard !state.compactStarting else { return .none }
                return compact(focus, stop: true, &state)

            case .compactDialog:
                return .none

            case .compactStarted:
                state.compactStarting = false
                return .none

            case .compactFailed(let focus, let message):
                state.compactStarting = false
                if !focus.isEmpty { putInBox("/compact \(focus)", ahead: true, &state) }
                state.alert = Self.alert(
                    "Couldn’t compact",
                    message + (focus.isEmpty ? "" : " What you asked it to keep in mind is back in the box."))
                return .none

            case .slashClosed:
                state.slashClosedOn = state.draft
                slashHidden(&state)
                return .none

            case .quoted(let text):
                let draft = state.draft.trimmingCharacters(in: .whitespacesAndNewlines)
                state.draft = draft.isEmpty ? text + "\n\n" : state.draft + "\n\n" + text + "\n\n"
                state.$savedDraft.withLock { $0 = state.draft }
                slashHidden(&state)
                return .none

            case .review(.delegate(.quote(let text))):
                return .send(.quoted(text))

            case .removePasteTapped(let id):
                state.pastes.removeAll { $0.id == id }
                let pastes = Paste.encode(state.pastes)
                state.$savedPastes.withLock { $0 = pastes }
                return .none

            case .pasteEdited(let id, let text):
                guard let index = state.pastes.firstIndex(where: { $0.id == id }) else { return .none }
                if text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    state.pastes.remove(at: index)
                } else {
                    state.pastes[index] = Paste(id: id, text: text)
                }
                let pastes = Paste.encode(state.pastes)
                state.$savedPastes.withLock { $0 = pastes }
                return .none

            case .pasteMovedToBox(let id, let text):
                guard let index = state.pastes.firstIndex(where: { $0.id == id }), Paste.fitsInBox(text) else {
                    return .none
                }
                state.pastes.remove(at: index)
                let pastes = Paste.encode(state.pastes)
                state.$savedPastes.withLock { $0 = pastes }
                putInBox(text, ahead: false, &state)
                return .none

            case .proposals(.delegate(.openSession(let session))):
                return .send(.delegate(.openSession(session)))

            case .itemTapped:
                guard let project = state.session.projectID, let item = state.session.itemID else { return .none }
                return .send(.delegate(.openItem(projectID: project, itemID: item)))

            case .specs(.delegate(.openItem(let item))), .planPanel(.delegate(.openItem(let item))):
                guard let project = state.session.projectID else { return .none }
                return .send(.delegate(.openItem(projectID: project, itemID: item)))

            case .alert(.dismiss):
                // The alert said an edit didn't reach its fork: now open it.
                guard let fork = state.unsentFork else { return .none }
                state.unsentFork = nil
                return .send(.forked(fork))

            case .binding, .delegate, .alert, .review, .docs, .specs, .planPanel, .terminals, .proposals, .land:
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
                // A watch that starts over asks again only for what isn't
                // known yet.
                let knowsImages = state.disconnected && state.readsImages != nil
                state.disconnected = false
                state.watchFailure = nil
                state.replaying = state.events.isEmpty
                // The history is sent again, and the reply's parts with it;
                // those already shown are skipped, so the reply stays.
                let session = state.session.id
                // A chat opened again, or a watch started over, has the
                // history up to its last event; only what follows is sent.
                let after = state.events.last?.id
                // Which chat commands the agent takes, once for a session
                // while Humata runs: again only if its host couldn't say.
                // An ask under way when the chat was last shown went with
                // that view's task, and its answer never comes: neither
                // flag may keep this one from asking.
                state.commandsLoading = false
                state.slashLoading = false
                let commands: Effect<Action> =
                    state.chatCommands == nil && !state.session.archived ? askCommands(&state) : .none
                // A chat that opens with its history already here gets no
                // proposing call to list its proposals on, so they're
                // listed now; one that opens empty is listed as its events
                // come.
                let proposals: Effect<Action> =
                    state.proposingCalls.isEmpty ? .none : .send(.proposals(.refresh))
                if !knowsImages, state.imagesCheckFailure != nil { state.imagesCheckFailure = nil }
                return .merge(
                    knowsImages ? .none : readsImages(&state),
                    commands,
                    proposals,
                    .run { send in
                        // The history goes as one batch once it has all
                        // come; after it, what comes while a batch is shown
                        // goes as the next one.
                        // A chat that opens empty has its history worked
                        // out there, off the main thread.
                        let batches = EventBatches(folds: after == nil)
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
                            case .events(let events, let folded, let replayed):
                                await send(.receivedAll(events, folded: folded))
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

            case .receivedAll(let events, let folded):
                state.historyLoaded = true
                state.historyLoading = nil
                // An empty history's end, or one said again: nothing to show.
                guard !events.isEmpty else { return .none }
                return received(events, folded: folded, &state)

            case .historyReplayed:
                state.replaying = false
                // The queued message whose edit was kept may have run
                // while the session wasn't shown.
                queuedEditLeftBehind(was: nil, &state)
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
                // Return with the / list showing picks from it.
                if state.slashOpen, state.slashChoices.indices.contains(state.slashSelected) {
                    return .send(.slashPicked(state.slashChoices[state.slashSelected].id))
                }
                // Return in the box with the /rewind picker over it picks
                // the highlighted point, as Return in the picker does.
                if let picker = state.rewindPicker, state.pastes.isEmpty, state.images.isEmpty,
                    state.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                    picker.points.indices.contains(picker.selected)
                {
                    return .send(.rewindPickerPicked(picker.points[picker.selected].id))
                }
                // A command only some hosts take waits until this one has
                // said whether it does, and stays in the box till then:
                // sent now, it would go to the agent as a message.
                if !state.commandsAnswered, !state.session.archived,
                    let command = SlashChoice.command(in: state.draft)?.command,
                    command == .compact || command == .side
                {
                    state.heldSend = HeldSend(draft: state.draft, mode: mode)
                    return askCommands(&state)
                }
                // One of Humata's own commands is carried out, not sent;
                // one the session isn't offered goes to the agent as typed.
                if let (command, text) = SlashChoice.command(in: state.draft), Self.offers(command, state) {
                    // Never sent to the agent as a message because of what
                    // else is in the box: a /side question would go to the
                    // agent that can write. The box stays as it is.
                    if command == .side, !state.images.isEmpty {
                        state.alert = Self.alert(
                            "/side takes no images",
                            "Nothing was sent. Take the images out of the box and press Return again, then add "
                                + "them in the side chat once it opens.")
                        return .none
                    }
                    if command != .side, !state.pastes.isEmpty || !state.images.isEmpty {
                        state.alert = Self.alert(
                            "/\(command.rawValue) takes no pasted text or images",
                            "Nothing was sent. Take the pasted text and images out of the box and press Return "
                                + "again; to send them to the agent instead, take the command out and put what "
                                + "you pasted in a message.")
                        return .none
                    }
                    switch command {
                    case .rewind:
                        rewindPickerOpened(text, &state)
                        return .none
                    case .compact:
                        // It stays in the box until it's under way, so
                        // cancelling leaves it there.
                        return .send(.compactAsked(focus: text))
                    case .newHere:
                        guard !state.forking else { return .none }
                        clearDraft(&state)
                        return .send(.childTapped(.empty(ownCopy: text.lowercased() == "copy"), message: ""))
                    case .fork:
                        guard !state.forking else { return .none }
                        clearDraft(&state)
                        return .send(.forkTapped(before: nil, chatOnly: text.lowercased() == "chat"))
                    case .side:
                        // It stays in the box until the side chat is made,
                        // so a failure doesn't lose the question.
                        guard !state.forking else { return .none }
                        // Pasted text is part of the question.
                        state.sidePastes = state.pastes.map(\.id)
                        return .send(
                            .childTapped(.side, message: Paste.message(draft: text, pastes: state.pastes)))
                    }
                }
                guard state.canSend else { return .none }
                let outgoing = Outgoing(
                    session: state.session.id, name: state.session.name, typed: state.draft, pastes: state.pastes,
                    images: Array(state.images), mode: mode,
                    text: Paste.message(draft: state.draft, pastes: state.pastes))
                state.draft = ""
                state.$savedDraft.withLock { $0 = "" }
                slashHidden(&state)
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
                    slashHidden(&state)
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

            case .readsImagesUnknown(let message):
                // What was last found still stands, and no image comes
                // out: the failure is only said while nothing is known.
                guard state.readsImages == nil else { return .none }
                if state.imagesCheckFailure != message { state.imagesCheckFailure = message }
                return .none

            case .readsImagesAsked:
                if state.imagesCheckFailure != nil { state.imagesCheckFailure = nil }
                return readsImages(&state)

            case .readsImagesFound(let reads):
                if state.imagesCheckFailure != nil { state.imagesCheckFailure = nil }
                if state.readsImages != reads { state.readsImages = reads }
                let found = ModelReadsImages(model: state.session.model, reads: reads)
                if state.savedReadsImages != found { state.$savedReadsImages.withLock { $0 = found } }
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
                let images = state.lastImages
                return .run { _ in
                    guard !images.isEmpty else { return try await daemon.send(session, text, .enqueue) }
                    // Its images go with it again; one that can't be read
                    // fails the retry, as the message isn't whole without.
                    var attachments: [ImageAttachment] = []
                    for name in images { attachments.append(try await daemon.readImage(session, name)) }
                    try await daemon.sendImages(session, text, attachments, .enqueue)
                } catch: { error, send in
                    await send(.failed(.retrying, error.localizedDescription))
                }

            case .moveApproved:
                guard !state.answeringRequest, let request = state.request, case .move = request.kind else {
                    return .none
                }
                state.answeringRequest = true
                state.requestAnswer = .moving
                return .send(.delegate(.answerRequest(request.id, .approve)))

            case .movePutOff:
                guard !state.answeringRequest, let request = state.request, case .move = request.kind else {
                    return .none
                }
                state.answeringRequest = true
                state.requestAnswer = .puttingOff
                return .send(.delegate(.answerRequest(request.id, .dismiss)))

            case .requestAllowed:
                guard !state.answeringRequest, let request = state.request, request.kind.asksForSession else {
                    return .none
                }
                state.answeringRequest = true
                state.requestAnswer = .allowing
                return .send(.delegate(.answerRequest(request.id, .approve)))

            case .requestDeclined:
                guard !state.answeringRequest, let request = state.request, request.kind.asksForSession else {
                    return .none
                }
                state.answeringRequest = true
                state.requestAnswer = .declining
                return .send(.delegate(.answerRequest(request.id, .dismiss)))

            case .showProposalsTapped:
                guard let row = state.firstWaitingProposalRow else {
                    // None of them is a row of this chat, as after the chat
                    // was cut back: the list is read again, so the strip
                    // says what waits now.
                    return .send(.proposals(.refresh))
                }
                state.scrollTarget = row
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
                // Its editor had the caret, and is gone.
                state.boxFocus += 1
                return .none

            case .editQueuedSaved:
                guard let id = state.editingQueued else { return .none }
                let text = state.queuedEdit.trimmingCharacters(in: .whitespacesAndNewlines)
                state.editingQueued = nil
                state.queuedEdit = ""
                state.boxFocus += 1
                let session = state.session.id
                let queued = state.queued.first(where: { $0.id == id })
                // Emptied, it's taken out of the queue; one that is only
                // images had no text to begin with, and stays.
                guard !text.isEmpty else {
                    return queued?.images.isEmpty == false ? .none : .send(.unqueueTapped(id))
                }
                guard queued?.text != text else { return .none }
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
                // Only the person's own message goes back in the box.
                guard state.isWorking, !state.isCompacting, !state.stopping, state.lastMessageOwn,
                    let text = state.lastMessage
                else { return .none }
                state.stopping = true
                let session = state.session.id
                putInBox(text, ahead: true, &state)
                let images = state.takesImages ? state.lastImages : []
                // Its images go back in the box with it.
                let reading: Effect<Action> =
                    images.isEmpty
                    ? .none
                    : .run { [daemon, uuid] send in
                        let read = await Self.sentImages(images, of: session, daemon, uuid)
                        if !read.isEmpty { await send(.sentImagesRead(read, rewound: false)) }
                    }
                return .merge(
                    .run { _ in
                        try await daemon.abort(session)
                    } catch: { error, send in
                        await send(.failed(.stopping, error.localizedDescription))
                    },
                    reading
                )

            case .rewindTapped(let id):
                guard let event = state.events[id: id], event.parentCallID == nil, !state.rewinding else {
                    return .none
                }
                let compaction: Bool
                // Another session's agent's message, or a notice of
                // Humata's, isn't the person's to edit.
                var own = true
                switch event.kind {
                case .userMessage(let text, _):
                    compaction = false
                    own = Chat.isPersons(text)
                case .compacted: compaction = true
                default: return .none
                }
                // The files only where the daemon rewinds them: never for
                // a side chat, and not in a copy that's another session's.
                let files =
                    state.session.projectID != nil && !state.session.archived && !state.session.readOnly
                    && !state.session.sharesCode
                let stops = state.isWorking ? "The agent’s turn stops and queued messages are dropped. " : ""
                let cuts =
                    compaction
                    ? "The agent gets back everything from before it was summed up, and what came after the "
                        + "compaction leaves the chat. "
                    : own
                        ? "Everything after it leaves the chat, and the message goes back in the box to edit. "
                        : "It and everything after it leave the chat. It wasn’t your message, so nothing goes "
                            + "back in the box. "
                let code =
                    files
                    ? "Rewinding files too puts the code back as it was then, uncommitted changes included. "
                    : ""
                let message = stops + cuts + code + "Undo brings it all back until you send another message."
                state.rewindDialog = ConfirmationDialogState {
                    TextState(compaction ? "Rewind to before this compaction?" : "Rewind to before this message?")
                } actions: {
                    ButtonState(action: .chat(id)) { TextState("Rewind the Chat") }
                    if files {
                        ButtonState(action: .chatAndFiles(id)) { TextState("Rewind the Chat and Files") }
                    }
                    ButtonState(role: .cancel) { TextState("Cancel") }
                } message: {
                    TextState(message)
                }
                return .none

            case .rewindDialog(.presented(let choice)):
                let (before, files) =
                    switch choice {
                    case .chat(let id): (id, false)
                    case .chatAndFiles(let id): (id, true)
                    }
                // A compaction puts nothing back in the box.
                let text: String
                var images: [String] = []
                switch state.events[id: before]?.kind {
                case .userMessage(let sent, let names):
                    // Only the person's own message goes back in the box.
                    if Chat.isPersons(sent) {
                        text = sent
                        if state.takesImages { images = names }
                    } else {
                        text = ""
                    }
                case .compacted: text = ""
                default: return .none
                }
                state.rewinding = true
                state.rewoundImages = []
                let session = state.session.id
                // Names for the images, only when there are some to name.
                let uuid: UUIDGenerator? = images.isEmpty ? nil : uuid
                return .run { [daemon] send in
                    // Read before the rewind, which takes the message out
                    // of the history its images belong to.
                    var read: [DraftImage] = []
                    if let uuid { read = await Self.sentImages(images, of: session, daemon, uuid) }
                    try await daemon.rewind(session, before, files)
                    if !read.isEmpty { await send(.sentImagesRead(read, rewound: true)) }
                    await send(.rewound(text))
                } catch: { error, send in
                    await send(.rewindFailed(error.localizedDescription))
                }

            case .rewindDialog:
                return .none

            case .rewound(let text):
                state.rewinding = false
                guard !text.isEmpty else {
                    state.rewoundText = nil
                    state.rewoundPaste = nil
                    return .none
                }
                state.rewoundText = text
                let paste = putInBox(text, ahead: true, &state)
                state.rewoundPaste = paste
                // The message is back in the box, to edit there.
                state.boxFocus += 1
                return .none

            case .sentImagesRead(let images, let rewound):
                // The model may have changed since they were sent.
                guard state.takesImages else { return .none }
                state.images = IdentifiedArray(uniqueElements: images + state.images)
                if rewound { state.rewoundImages = images.map(\.id) }
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
                    slashHidden(&state)
                }
                for image in state.rewoundImages { state.images.remove(id: image) }
                state.rewoundImages = []
                state.rewoundText = nil
                state.rewoundPaste = nil
                return .none

            case .rewindFailed(let message):
                state.rewinding = false
                // The daemon says why and what to do about it.
                state.alert = Self.alert("Couldn’t rewind", message)
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

            case .model(let action):
                return modelSwitch(&state, action)

            case .plan(let action):
                return planSwitch(&state, action)

            case .thinkingToggled(let id):
                state.expandedThinking.formSymmetricDifference([id])
                return .none

            case .toolToggled(let id):
                state.expandedTools.formSymmetricDifference([id])
                return .none

            case .subagentTapped(let callID):
                guard state.subagents[id: callID] != nil else { return .none }
                state.shownSubagent = callID
                return .none

            case .subagentClosed:
                state.shownSubagent = nil
                return .none

            case .forkTapped(let before, let asked):
                guard !state.forking else { return .none }
                state.forking = true
                let session = state.session
                // A copy of the code would be one a read-only session's
                // fork, read-only too, could never write.
                let chatOnly = asked || session.readOnly
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
                guard case .userMessage(let text, let images)? = state.events[id: id]?.kind, Chat.isPersons(text)
                else {
                    return .none
                }
                state.editing = id
                state.edited = text
                state.editedImages = images
                state.editCaret = true
                return .none

            case .editCaretTaken:
                if state.editCaret { state.editCaret = false }
                return .none

            case .editCancelled:
                state.editing = nil
                state.edited = ""
                state.editedImages = []
                state.editCaret = false
                // Its editor had the caret, and is gone.
                state.boxFocus += 1
                return .none

            case .editSent:
                let text = state.edited.trimmingCharacters(in: .whitespacesAndNewlines)
                guard let before = state.editing, !text.isEmpty, !state.forking else { return .none }
                // The edit stays open until the fork is made, so a failure
                // doesn't lose it.
                state.forking = true
                let session = state.session
                let images = state.editedImages
                return .run { send in
                    // Its images are read before the fork is made: the
                    // message isn't whole without one, so one that can't
                    // be read makes no fork.
                    var attachments: [ImageAttachment] = []
                    for name in images { attachments.append(try await daemon.readImage(session.id, name)) }
                    // Chat only for a read-only session, as any fork of it.
                    let fork = try await daemon.fork(session.id, before, "\(session.name) edit", session.readOnly)
                    do {
                        if attachments.isEmpty {
                            try await daemon.send(fork.id, text, .enqueue)
                        } else {
                            try await daemon.sendImages(fork.id, text, attachments, .enqueue)
                        }
                    } catch {
                        await send(
                            .forkSendFailed(
                                fork: fork, text: text, images: attachments.count,
                                message: error.localizedDescription))
                        return
                    }
                    await send(.forked(fork))
                } catch: { error, send in
                    await send(.failed(.forking, error.localizedDescription))
                }

            case .forked(let fork):
                state.forking = false
                state.makingChild = nil
                state.editing = nil
                state.edited = ""
                state.editedImages = []
                state.editCaret = false
                return .send(.delegate(.openSession(fork)))

            case .forkSendFailed(let fork, let text, let images, let message):
                // The fork's box keeps the text, to send from there.
                let draft = Shared(wrappedValue: "", .appStorage(State.draftKey(fork.id)))
                draft.withLock { $0 = $0.isEmpty ? text : $0 + "\n\n" + text }
                // The fork is made, so the edit is done with here.
                state.forking = false
                state.makingChild = nil
                state.editing = nil
                state.edited = ""
                state.editedImages = []
                state.editCaret = false
                // Said here first, where the edit was made: the fork opens
                // when the alert is dismissed.
                state.unsentFork = fork
                var words =
                    "The fork “\(fork.name)” was made, but your edited message didn’t reach it: \(message) "
                    + "Its text is in the fork’s message box, which opens next: send it from there."
                if images > 0 {
                    words +=
                        images == 1
                        ? " Its image didn’t go with it and isn’t in the box: add it again there before you send."
                        : " Its \(images) images didn’t go with it and aren’t in the box: add them again there "
                            + "before you send."
                }
                state.alert = Self.alert("The edited message wasn’t sent", words)
                return .none

            case .childTapped(let asked, let message):
                guard !state.forking, !state.session.archived else { return .none }
                let session = state.session
                // No copy of its own under a read-only session: the chat
                // would be read-only too, and could never write to it.
                let kind = session.readOnly && asked == .empty(ownCopy: true) ? .empty(ownCopy: false) : asked
                state.forking = true
                state.makingChild = kind
                let name = kind == .side ? "\(session.name) side" : "New chat"
                let text = message.trimmingCharacters(in: .whitespacesAndNewlines)
                return .run { send in
                    let child = try await daemon.newChild(session.id, name, kind)
                    guard kind == .side else {
                        await send(.forked(child))
                        return
                    }
                    var unsent = ""
                    var failure = ""
                    if !text.isEmpty {
                        do {
                            try await daemon.send(child.id, text, .enqueue)
                        } catch {
                            unsent = text
                            failure = error.localizedDescription
                        }
                    }
                    await send(.sideMade(child, unsent: unsent, failure: failure))
                } catch: { error, send in
                    await send(.failed(.startingChild, error.localizedDescription))
                }

            case .sideMade(let side, let unsent, let failure):
                state.forking = false
                state.makingChild = nil
                if SlashChoice.command(in: state.draft)?.command == .side { clearDraft(&state) }
                // The pastes that went with the question go with it.
                if !state.sidePastes.isEmpty {
                    let sent = Set(state.sidePastes)
                    state.sidePastes = []
                    if state.pastes.contains(where: { sent.contains($0.id) }) {
                        state.pastes.removeAll { sent.contains($0.id) }
                        let pastes = Paste.encode(state.pastes)
                        state.$savedPastes.withLock { $0 = pastes }
                    }
                }
                if !unsent.isEmpty {
                    // The side chat's box keeps it, to send from there.
                    let draft = Shared(wrappedValue: "", .appStorage(State.draftKey(side.id)))
                    draft.withLock { $0 = $0.isEmpty ? unsent : $0 + "\n\n" + unsent }
                    state.alert = Self.alert(
                        "The side chat is open, but your question didn’t reach it",
                        (failure.isEmpty ? "" : failure + " ")
                            + "The question is in the side chat’s box: send it from there.")
                }
                return .send(.delegate(.openSide(side)))

            case .failed(let doing, let message):
                switch doing {
                case .updating: state.updating = false
                case .forking, .startingChild:
                    state.forking = false
                    state.makingChild = nil
                    state.sidePastes = []
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
        .ifLet(\.$compactDialog, action: \.compactDialog)
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
        slashHidden(&state)
        return nil
    }

    private static func title(_ doing: Action.Doing) -> String {
        switch doing {
        case .stopping: "Couldn’t stop the turn"
        case .retrying: "Couldn’t send your message again"
        case .unqueueing: "Couldn’t take the message out of the queue"
        case .forking: "Couldn’t fork the session"
        case .startingChild: "Couldn’t start the chat"
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
///
/// With `folds`, the history's chat is worked out here as it's handed on:
/// on this actor, so off the main thread, which would otherwise stand
/// still for as long as a long history takes to work out.
private actor EventBatches {
    enum Batch: Sendable {
        /// `replayed` when they end the history the watch starts with,
        /// and `folded` then holds its chat, if it was to be worked out.
        case events([SessionEvent], folded: FoldedHistory?, replayed: Bool)
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
    private let folds: Bool

    init(folds: Bool) {
        self.folds = folds
    }

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
            let folded = folds && !history.isEmpty ? FoldedHistory(history) : nil
            return .events(history, folded: folded, replayed: true)
        }
        if !pending.isEmpty {
            defer { pending = [] }
            return .events(pending, folded: nil, replayed: false)
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

/// The chat a history shows, worked out before the history reaches the
/// chat it's for, so that isn't done on the main thread.
package struct FoldedHistory: Sendable {
    let chat: Chat
    let subagents: SubagentLog
    /// How many events it was worked out from: all but a reply's parts as
    /// it streamed, which the chat doesn't keep.
    let count: Int
    /// Those events as the chat keeps them, by id, so a chat that takes
    /// the history whole builds nothing on the main thread.
    let log: IdentifiedArrayOf<SessionEvent>
    /// Whether no event came twice. Only then are `log` and what follows
    /// what a chat with nothing yet comes to.
    let unique: Bool
    /// About how much memory the log holds.
    let bytes: Int
    /// The reply still being written where the history ends, and its
    /// parts taken in.
    let streaming: String
    let streamed: Set<String>
    /// The last rewind or undo of one in it.
    let restart: String?
    let turnStarted: Bool
    let turnEnded: Bool
    /// A tool finished or a turn ended, so the code may have changed.
    let changedCode: Bool
    /// A call proposing a task or a spec edit finished well.
    let proposed: Bool

    init(_ events: [SessionEvent]) {
        var chat = Chat()
        var subagents = SubagentLog()
        var count = 0
        var log: IdentifiedArrayOf<SessionEvent> = []
        var unique = true
        var bytes = 0
        var streaming = ""
        var streamed: Set<String> = []
        var restart: String?
        var turnStarted = false
        var turnEnded = false
        var toolCompleted = false
        for event in events {
            if case .assistantDelta(let text) = event.kind {
                // A subagent's words aren't the reply's.
                if event.parentCallID == nil, streamed.insert(event.id).inserted { streaming += text }
                continue
            }
            SessionFeature.State.fold(event, into: &chat, &subagents)
            count += 1
            guard log.append(event).inserted else {
                unique = false
                continue
            }
            bytes += SessionFeature.State.bytes(of: event)
            switch event.kind {
            case .assistantMessage where event.parentCallID == nil:
                // The reply's whole message ends the one being written.
                streaming = ""
                streamed = []
            case .turnEnded:
                streaming = ""
                streamed = []
                turnEnded = true
            case .turnStarted: turnStarted = true
            case .toolCompleted: toolCompleted = true
            case .rewound, .rewindUndone: restart = event.id
            default: break
            }
        }
        self.chat = chat
        self.subagents = subagents
        self.count = count
        self.log = log
        self.unique = unique
        self.bytes = bytes
        self.streaming = streaming
        self.streamed = streamed
        self.restart = restart
        self.turnStarted = turnStarted
        self.turnEnded = turnEnded
        changedCode = toolCompleted || turnEnded
        let proposals = chat.proposals
        proposed =
            !proposals.isEmpty
            && events.contains { event in
                if case .toolCompleted(let call, true, _) = event.kind { proposals.contains(call) } else { false }
            }
    }
}

/// Whether a session's model was found to see images, with the model it
/// was found for: the session may be given another since.
struct ModelReadsImages: Equatable, Sendable {
    var model: String?
    var reads: Bool
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
