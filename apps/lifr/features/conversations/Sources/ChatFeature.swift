import CaptureKit
import ChatFeatures
import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrCore
import LifrModelsClient
import LifrSkills
import LifrStoreClient

/// A chat with the assistant: it answers questions, proposes changes to
/// tasks and Projects when asked, which the person applies, edits or
/// declines (each reply applied has Undo), and drafts plans on a card the
/// person can edit and then add. A reply keeps going when the person
/// leaves the chat or Lifr; one the system turns away, as Apple's model
/// does in the background, is sent again on `resume`.
///
/// The shared chat writes, shows and goes between the messages; this adds
/// what Lifr keeps with them: the records, skills, recordings, the draft,
/// what replies change, and their summaries.
@Reducer
package struct ChatFeature {
    package typealias Core = ChatCore<LifrReplier>

    @ObservableState
    package struct State: Equatable, Identifiable {
        /// The shared chat, which keeps the conversation, the field and the
        /// reply being written.
        package var core: Core.State
        /// The latest save shown, so an earlier one's late reply is ignored.
        package var saves = SaveOrder()
        package var availability = AssistantAvailability.available
        /// Whether the system turned the reply away, such as while Lifr was
        /// in the background, so `resume` sends it again.
        package var stopped = false
        /// Whether the changes of the last reply are still being looked up.
        package var reloading = false
        /// Whether the assistant is naming the conversation.
        package var naming = false
        @Shared(.namesConversations) package var namesConversations
        @Shared(.picksSkills) package var picksSkills
        @Shared(.compactsConversations) package var compactsConversations
        /// How many older messages the model didn't get with the last
        /// message, as they didn't fit: the chat then offers Compact.
        package var leftOut = 0
        /// Whether the assistant is summarizing older messages.
        package var compacting = false
        /// The day the draft's dates are read from, such as "saturday".
        package var today: CalendarDate?
        @Presents package var alert: AlertState<Action.Alert>?
        @Presents package var manageSkills: SkillsFeature.State?
        /// What the conversation runs on, being picked.
        @Presents package var model: ChatModelFeature.State?
        /// Whether the draft's editor is open.
        package var editsDraft = false
        /// The draft as it was when its editor opened, put back on Cancel.
        package var draftBeforeEdit: Plan?
        /// The draft last discarded, put back on Undo until the next
        /// message is sent.
        package var discardedDraft: Plan?
        /// The drafts this chat added, by the journal entry that added
        /// them: undone, the draft is back to add again.
        package var addedDrafts: [UUID: Plan] = [:]
        /// The proposal being edited, before it's checked again.
        package var editingProposal: ProposalEdit?
        /// The reply whose proposal is being applied or checked again.
        package var applying: UUID?
        /// What the assistant's model reads besides text: the microphone
        /// records for one that listens, and dictates otherwise.
        package var inputs: Set<ModelInput> = []
        /// Whether the microphone is recording for the next message, or
        /// starting to.
        package var recording = false
        /// Whether the microphone has started recording, rather than still
        /// starting, as while the system asks for it.
        package var recordingLive = false
        /// Recordings of this conversation whose files aren't on this
        /// device, such as those sent from another one.
        package var elsewhere: Set<UUID> = []
        /// The file of the sent recording being played.
        package var playing: URL?
        /// Whether the model was asked to load, once the person started
        /// writing. Opening a chat doesn't: Lifr reopens the last one at
        /// launch, and a model takes gigabytes.
        package var warmed = false
        /// Whether the log of every point to go back or forward to is open.
        package var showsCheckpoints = false

        package init(conversation: Conversation, workspace: Workspace) {
            core = Core.State(
                thread: conversation, model: conversation.context.model, use: .assistant, instructions: "",
                host: LifrHost(workspace: workspace))
            forgetUndone()
            sync()
        }

        package var id: UUID { core.id }

        package var conversation: Conversation {
            get { core.thread }
            set { core.thread = newValue }
        }

        package var workspace: Workspace {
            get { core.host.workspace }
            set { core.host.workspace = newValue }
        }

        package var input: String {
            get { core.input }
            set { core.input = newValue }
        }

        /// Why the draft can't be added as it is now, checked on every
        /// change, and sent with the next message.
        package var problem: String? {
            get { core.host.problem }
            set { core.host.problem = newValue }
        }

        /// Every skill, on or off. The built-ins until the person's load.
        package var skills: [Skill] {
            get { core.host.skills }
            set { core.host.skills = newValue }
        }

        /// The ID of the skill picked for the next message, shown as its
        /// "/name" at the start of the field.
        package var skill: String? {
            get { core.host.skill }
            set { core.host.skill = newValue }
        }

        /// The recordings to send with the next message.
        package var recordings: [ChatAttachment] {
            get { core.host.recordings }
            set { core.host.recordings = newValue }
        }

        /// Shows `snapshot`'s records, letting go of replies' changes they
        /// undid elsewhere, such as with the Undo toast or ⌘Z, unless later
        /// ones are shown already.
        package mutating func show(_ snapshot: Snapshot) {
            guard saves.isLatest(snapshot) else { return }
            workspace = snapshot.workspace
            forgetUndone()
        }

        /// Lets go of replies' changes that were all undone, so they don't
        /// offer Undo again, and puts back a draft whose add was undone.
        private mutating func forgetUndone() {
            guard !conversation.changes.isEmpty || !addedDrafts.isEmpty else { return }
            let undone = workspace.undoneEntries
            for (message, entries) in conversation.changes where undone.isSuperset(of: entries) {
                conversation.changes[message] = nil
            }
            if conversation.draft == nil, let entry = addedDrafts.keys.first(where: undone.contains) {
                conversation.draft = addedDrafts.removeValue(forKey: entry)
            }
        }

        /// Tells the shared chat what Lifr holds it for, and what it runs
        /// on. The reducer does it on each action; a state made for a
        /// preview or snapshot does it once made.
        package mutating func sync() {
            core.held = reloading || compacting
            core.canReply = availability == .available && !recording
            core.sendsEmpty = picked != nil || !recordings.isEmpty
            core.model = conversation.context.model
        }

        /// Whether a reply is being written, waits to be sent again, is
        /// being finished, or failed: the chat is kept until it isn't.
        package var isWorking: Bool {
            core.reply != nil || core.rival?.done == false || stopped || reloading || naming || compacting
                || alert != nil || core.problem != nil || recording || core.attaching
        }

        /// Whether the microphone records a clip rather than dictating: for
        /// a model that listens, unless it's dictating already, and while
        /// it records, even once another model is picked, so it can stop.
        package var records: Bool {
            recording || (inputs.contains(.audio) && !core.dictating && core.dictatedAfter == nil)
        }

        /// Whether older messages can be summarized now: there are some
        /// after the newest summary, and no reply is being written.
        package var canCompact: Bool {
            availability == .available && core.reply == nil && !compacting && !conversation.compactable.isEmpty
        }

        /// Whether to offer Compact: the last reply was written without
        /// some of the conversation.
        package var offersCompact: Bool { leftOut > 0 && canCompact }

        /// Whether the draft can be added, edited or discarded: not while a
        /// reply is still writing it.
        package var canChangeDraft: Bool { conversation.draft != nil && core.reply == nil }

        package var canAdd: Bool { canChangeDraft && problem == nil }

        /// Whether the conversation can go back, forward or to another
        /// branch: not while a reply is written or its changes are read.
        package var canGo: Bool { core.canGo }

        /// Whether the conversation can be forked at the message `id`: also
        /// while a reply is written, from before the message it answers.
        package func canFork(at id: UUID) -> Bool {
            guard !conversation.messages.isEmpty else { return false }
            return canGo || id != conversation.messages.last?.id
        }

        /// Whether a reply is being written, which Stop ends.
        package var canStop: Bool { core.reply != nil }

        package var canSend: Bool { core.canSend }

        /// The skill picked for the next message, while it exists and is on.
        package var picked: Skill? { skill.flatMap { id in skills.first { $0.id == id && $0.isOn } } }

        /// The skills that are on, then the commands that can run now,
        /// matching what's typed after a / that starts the message, while
        /// no skill is picked. Empty without a /.
        package var slashChoices: [SlashChoice] {
            guard picked == nil, input.hasPrefix("/") else { return [] }
            let query = String(input.dropFirst())
            let commands = ChatCommand.allCases.filter {
                $0.isAvailable(in: self) && (query.isEmpty || $0.name.hasPrefix(query.lowercased()))
            }
            let choices =
                Skill.search(query, in: skills.filter(\.isOn)).map(SlashChoice.skill)
                + commands.map(SlashChoice.command)
            // Names that start with what's typed come before looser matches,
            // so a command isn't pushed out of sight by skills.
            let typed = query.lowercased()
            func starts(_ choice: SlashChoice) -> Bool {
                let name = choice.id.lowercased()
                return name.hasPrefix(typed) || name.hasPrefix(Skill.userPrefix + typed)
            }
            return choices.filter(starts) + choices.filter { !starts($0) }
        }

        /// The skill that's on, or the command that can run now, typed as
        /// `name` after a /, ignoring case. A skill of the person's goes by
        /// its name with or without "user:".
        func slash(named name: String) -> SlashChoice? {
            let name = name.lowercased()
            if let command = ChatCommand(rawValue: name), command.isAvailable(in: self) {
                return .command(command)
            }
            let skill = skills.first { skill in
                skill.isOn && (skill.id.lowercased() == name || skill.id.lowercased() == Skill.userPrefix + name)
            }
            return skill.map(SlashChoice.skill)
        }

        /// What the reply `id` proposed, if it's shown.
        package func proposal(of id: UUID) -> Proposal? {
            conversation.messages.first { $0.id == id }?.proposal
        }

        /// Puts `proposal` on the reply `id` in place of what it proposed.
        mutating func setProposal(_ proposal: Proposal, of id: UUID) {
            guard let index = conversation.messages.firstIndex(where: { $0.id == id }) else { return }
            conversation.messages[index].proposal = proposal
        }

        /// Keeps what the person decided about the reply's proposal.
        mutating func decide(_ id: UUID, _ decision: Proposal.Decision) {
            guard var proposal = proposal(of: id) else { return }
            proposal.decision = decision
            setProposal(proposal, of: id)
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// The shared chat: most of what the person does in it.
        case core(Core.Action)
        case skillsLoaded([Skill])
        case pickSkillTapped(String)
        /// A command picked from the / list, which runs now.
        case pickCommandTapped(ChatCommand)
        case clearSkillTapped
        case manageSkillsTapped
        case manageSkills(PresentationAction<SkillsFeature.Action>)
        case model(PresentationAction<ChatModelFeature.Action>)
        /// A suggestion in an empty chat: sent as the person's message.
        case suggestionTapped(String)
        case editDraftTapped
        /// Closes the draft's editor, putting back the draft it opened on.
        case cancelEditDraftTapped
        case cancelRecordingTapped
        case recordingStarted(Result<Void, any Error>)
        /// The longest recording was reached.
        case recordingLimitReached
        case recorded(Result<ChatAttachment, any Error>)
        case removeRecordingTapped(UUID)
        /// A sent recording's chip: plays it.
        case recordingTapped(UUID)
        /// Lifr is back on screen: send a stopped reply's request again.
        case resume
        case reloaded(messageID: UUID, Result<Snapshot, any Error>)
        case named(Result<String, any Error>)
        /// The title was tapped, to rename the conversation.
        case renameTapped
        /// Summarizes the messages before the last two, to send in their
        /// place.
        case compactTapped
        case compacted(through: UUID, Result<String, any Error>)
        /// Removes the summary that ends at the message, so the messages it
        /// covers are sent again as far as they fit.
        case forgetSummaryTapped(UUID)
        case undoTapped(messageID: UUID)
        /// Saves what the reply proposed, made again on the records as they
        /// are now, as one change.
        case applyTapped(messageID: UUID)
        case applied(messageID: UUID, Result<Saved, any Error>)
        case declineTapped(messageID: UUID)
        case editProposalTapped(messageID: UUID)
        /// Checks the edited proposal against the records, and keeps it
        /// in place of the reply's when it can be made.
        case editProposalDone
        case reproposed(messageID: UUID, Result<Proposal, any Error>)
        case discardDraftTapped
        case undoDiscardTapped
        case addTapped
        case saved(Result<Saved, any Error>)
        /// Goes to the point before the last one shown.
        case backTapped
        /// Goes to the point after the last one shown, the way it last went.
        case forwardTapped
        /// A point in the log, or a message, to go to.
        case goTapped(UUID)
        /// Makes a new conversation of the messages up to this one.
        case forkTapped(messageID: UUID)
        case forked(Conversation)
        /// Sends the message being edited, once the person said whether to
        /// undo the changes of the replies that hides.
        case sendConfirmed
        /// Writes the reply to the message `id`, or the reply `id`, again,
        /// once the person said whether to undo the changes of the replies
        /// that hides.
        case retryConfirmed(UUID)
        case alert(PresentationAction<Alert>)
        case delegate(Delegate)

        package enum Alert: Equatable {
            /// Goes to the message, undoing the changes of the replies
            /// that hides or keeping them.
            case go(to: UUID, undo: Bool)
            /// Sends the message being edited in place of the old one.
            case resend(undo: Bool)
            /// Writes the reply to the message, or the reply, again.
            case retry(UUID, undo: Bool)
        }

        @CasePathable
        package enum Delegate {
            case saved(Snapshot)
            case rename
            /// Opens a new conversation, as /new does.
            case new
            /// A new conversation forked from this one, to keep and open.
            case fork(Conversation)
        }
    }

    /// A reply's proposal as the person edits it: each change can be left
    /// out or changed.
    package struct ProposalEdit: Equatable, Identifiable, Sendable {
        /// The reply.
        package var id: UUID
        package var actions: [AssistantAction]
        /// Whether each of `actions` is kept.
        package var kept: [Bool]
        /// Why the edited changes can't be made, shown in the editor.
        package var problem: String?

        package init(id: UUID, actions: [AssistantAction]) {
            self.id = id
            self.actions = actions
            kept = actions.map { _ in true }
        }

        /// The actions kept.
        var chosen: [AssistantAction] {
            zip(actions, kept).compactMap { action, kept in kept ? action : nil }
        }
    }

    private enum CancelID { case recording }

    @Dependency(\.assistant) private var assistant
    @Dependency(\.store) private var store
    @Dependency(\.skills) private var skillsClient
    @Dependency(\.attachments) private var attachments
    @Dependency(\.recorder) private var recorder
    @Dependency(\.continuousClock) private var clock
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone
    @Dependency(\.uuid) private var uuid

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, _ in
            state.sync()
            return .none
        }
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .core(.task):
                state.availability = assistant.availability(state.conversation.context.model)
                state.today = CalendarDate(now, in: timeZone)
                state.inputs = assistant.inputs(state.conversation.context.model)
                // A draft kept from before is checked against the records
                // as they are now, so Add says whether it can add it.
                if let draft = state.conversation.draft {
                    let repaired = draft.repaired(for: state.workspace)
                    if !repaired.isEmpty { state.conversation.draft = repaired }
                }
                check(&state)
                let conversation = state.conversation.id
                state.elsewhere = Set(
                    state.conversation.messages.flatMap(\.attachments)
                        .filter { $0.kind == .audio && attachments.file($0, conversation) == nil }.map(\.id))
                return .merge(
                    core(.task, &state),
                    .run { [skillsClient] send in
                        // Without the person's skills, the built-ins still work.
                        if let skills = try? await skillsClient.load() { await send(.skillsLoaded(skills)) }
                    })

            case .skillsLoaded(let skills):
                state.skills = skills
                return .none

            case .pickSkillTapped(let id):
                guard state.skills.contains(where: { $0.id == id && $0.isOn }) else { return .none }
                state.skill = id
                // Picked from what was typed after a /, which it replaces.
                if state.input.hasPrefix("/") { state.input = "" }
                return .none

            case .clearSkillTapped:
                state.skill = nil
                return .none

            case .pickCommandTapped(let command):
                guard command.isAvailable(in: state) else { return .none }
                // Picked from what was typed after a /, which it replaces.
                if state.input.hasPrefix("/") { state.input = "" }
                return run(command, details: "", in: &state)

            case .manageSkillsTapped:
                state.manageSkills = SkillsFeature.State()
                return .none

            case .manageSkills(.dismiss):
                return .run { [skillsClient] send in
                    if let skills = try? await skillsClient.load() { await send(.skillsLoaded(skills)) }
                }

            case .core(.modelTapped):
                state.model = ChatModelFeature.State(
                    choice: state.conversation.context.model, use: state.core.use,
                    picksSkills: state.conversation.context.picksSkills, standardPicksSkills: state.picksSkills)
                return .none

            case .model(.presented):
                if let sheet = state.model, !sheet.models.isEmpty {
                    state.core.models = sheet.models
                    state.core.standard = sheet.standard
                }
                if let sheet = state.model, sheet.picksSkills != state.conversation.context.picksSkills {
                    state.conversation.context.picksSkills = sheet.picksSkills
                }
                // Each change applies to the next message.
                guard let choice = state.model?.choice, choice != state.conversation.context.model else {
                    return .none
                }
                if choice.model != state.conversation.context.model.model
                    || choice.window != state.conversation.context.model.window
                {
                    state.core.context = nil
                }
                state.conversation.context.model = choice
                state.availability = assistant.availability(choice)
                state.inputs = assistant.inputs(choice)
                // The new model loads once the person writes.
                state.warmed = false
                return .none

            case .core(.sendTapped):
                // "/add buy milk" picks the skill and sends "buy milk".
                if let typed = takeSlash(in: &state, complete: true) {
                    return run(typed.command, details: typed.details, in: &state)
                }
                state.sync()
                guard state.canSend else { return .none }
                if let editing = state.core.editing {
                    let changed = state.conversation.changedReplies(hiddenGoingBefore: editing)
                    guard changed.isEmpty else {
                        state.alert = notice(
                            changed, in: state, undo: .resend(undo: true), keep: .resend(undo: false))
                        return .none
                    }
                }
                return send(in: &state)

            case .sendConfirmed:
                state.sync()
                guard state.canSend else { return .none }
                return send(in: &state)

            case .core(.editTapped(let id)):
                let effect = core(.editTapped(messageID: id), &state)
                if state.core.editing == id {
                    state.skill = state.conversation.messages.first { $0.id == id }?.skill
                }
                return effect

            case .core(.cancelEditTapped):
                state.skill = nil
                return core(.cancelEditTapped, &state)

            case .core(.retryTapped(let id)):
                let messages = state.conversation.messages
                guard let index = messages.firstIndex(where: { $0.id == id }), state.core.canRetry(messages[index]),
                    let asked = messages[...index].last(where: { $0.role == .person })
                else { return .none }
                let changed = state.conversation.changedReplies(hiddenGoingTo: asked.id)
                guard changed.isEmpty else {
                    state.alert = notice(
                        changed, in: state, undo: .retry(id, undo: true), keep: .retry(id, undo: false))
                    return .none
                }
                return .send(.retryConfirmed(id))

            case .retryConfirmed(let id):
                state.core.host.pick = nil
                return core(.retryTapped(messageID: id), &state)

            case .alert(.presented(.resend(let undo))):
                guard let editing = state.core.editing else { return .none }
                let changed = undo ? state.conversation.changedReplies(hiddenGoingBefore: editing) : []
                return undoing(changed, in: state) { .sendConfirmed }

            case .alert(.presented(.retry(let id, let undo))):
                let messages = state.conversation.messages
                guard let index = messages.firstIndex(where: { $0.id == id }),
                    let asked = messages[...index].last(where: { $0.role == .person })
                else { return .none }
                let changed = undo ? state.conversation.changedReplies(hiddenGoingTo: asked.id) : []
                return undoing(changed, in: state) { .retryConfirmed(id) }

            case .suggestionTapped(let text):
                guard state.conversation.messages.isEmpty else { return .none }
                state.input = text
                return .send(.core(.sendTapped))

            case .editDraftTapped:
                guard state.canChangeDraft else { return .none }
                state.editsDraft = true
                state.draftBeforeEdit = state.conversation.draft
                return .none

            case .cancelEditDraftTapped:
                guard state.editsDraft else { return .none }
                state.editsDraft = false
                if let draft = state.draftBeforeEdit {
                    state.conversation.draft = draft
                    check(&state)
                }
                state.draftBeforeEdit = nil
                return .none

            case .core(.micTapped) where state.records:
                guard state.recording else {
                    guard !state.core.attaching else { return .none }
                    state.recording = true
                    state.recordingLive = false
                    return .run { [recorder, clock] send in
                        do {
                            try await recorder.start()
                        } catch {
                            await send(.recordingStarted(.failure(error)))
                            return
                        }
                        await send(.recordingStarted(.success(())))
                        // The recorder stops by itself at the limit; what it
                        // heard is attached as if the person had stopped it.
                        try await clock.sleep(for: .seconds(RecorderClient.limit))
                        await send(.recordingLimitReached)
                    }
                    .cancellable(id: CancelID.recording, cancelInFlight: true)
                }
                // Stopped while the microphone was still starting, as while
                // the system asks for it: there's nothing to keep.
                guard state.recordingLive else { return cancelRecording(&state) }
                return stopRecording(&state)

            case .recordingLimitReached:
                guard state.recording, state.recordingLive else { return .none }
                return stopRecording(&state)

            case .recordingStarted(.success):
                state.recordingLive = state.recording
                return .none

            case .recordingStarted(.failure(let error)):
                state.recording = false
                state.recordingLive = false
                state.alert = Self.attachAlert(error)
                return .none

            case .cancelRecordingTapped:
                guard state.recording else { return .none }
                return cancelRecording(&state)

            case .recorded(.success(let recording)):
                state.core.attaching = false
                state.recordings.append(recording)
                return .none

            case .recorded(.failure(let error)):
                state.core.attaching = false
                state.alert = Self.attachAlert(error)
                return .none

            case .removeRecordingTapped(let id):
                guard let recording = state.recordings.first(where: { $0.id == id }) else { return .none }
                state.recordings.removeAll { $0.id == id }
                let conversation = state.conversation.id
                return .run { [attachments] _ in await attachments.remove(recording, conversation) }

            case .recordingTapped(let id):
                guard
                    let recording = state.conversation.messages.lazy.flatMap(\.attachments)
                        .first(where: { $0.id == id && $0.kind == .audio })
                else { return .none }
                state.playing = attachments.file(recording, state.conversation.id)
                return .none

            case .core(.stopTapped):
                state.stopped = false
                return core(.stopTapped, &state)

            case .resume:
                // A reply still being written carries on by itself.
                guard state.stopped, let written = state.core.reply, let asked = state.conversation.messages.last
                else { return .none }
                state.stopped = false
                let thinking = state.core.replyThinking
                state.core.reply = nil
                state.core.host.pick = nil
                let effect = core(.retryTapped(messageID: asked.id), &state)
                // What was written before it was turned away stays until
                // the reply carries on.
                if state.core.reply != nil {
                    state.core.reply = written
                    state.core.replyThinking = thinking
                }
                return effect

            case .core(.replied(.failure(.assistant(let error)))) where error == .busy:
                // Keep what's written so far, and send again on `resume`.
                state.stopped = true
                return .none

            case .core(.replied(.success(let reply))):
                let effect = core(.replied(.success(reply)), &state)
                guard let id = state.conversation.messages.last?.id,
                    state.conversation.messages.last?.role == .assistant
                else { return effect }
                if let skill = reply.extra?.skill,
                    let asked = state.conversation.messages.dropLast().lastIndex(where: { $0.role == .person })
                {
                    state.conversation.messages[asked].skill = skill
                }
                return .merge(effect, finish(reply.extra, of: id, in: &state))

            case .core(.keepTapped(let keepsRival)):
                let extra = state.core.rival?.extra
                let effect = core(.keepTapped(rival: keepsRival), &state)
                if keepsRival, let plan = extra?.plan { draft(plan, in: &state) }
                return effect

            case .named(let result):
                state.naming = false
                // A failed name leaves the first message as the title.
                guard case .success(let name) = result, !state.conversation.isNamedByHand else { return .none }
                state.conversation.name = name
                return .none

            case .renameTapped:
                return .send(.delegate(.rename))

            case .compactTapped:
                guard state.canCompact else { return .none }
                return compact(&state)

            case .compacted(let through, .success(let text)):
                state.compacting = false
                state.conversation.summaries[through] = ChatSummary(text: text, at: now)
                state.leftOut = 0
                return .none

            case .compacted(_, .failure(let error)):
                state.compacting = false
                state.alert = AlertState {
                    TextState("Couldn't compact")
                } actions: {
                    ButtonState(role: .cancel) { TextState("OK") }
                } message: {
                    TextState(
                        "\(String(describing: error)) The conversation is as it was: the newest messages that fit "
                            + "are still sent.")
                }
                return .none

            case .forgetSummaryTapped(let id):
                guard !state.compacting else { return .none }
                state.conversation.summaries[id] = nil
                return .none

            case .backTapped:
                guard let back = state.conversation.back else { return .none }
                return go(to: back, in: &state)

            case .forwardTapped:
                guard let forward = state.conversation.forward else { return .none }
                return go(to: forward, in: &state)

            case .core(.latestTapped):
                guard let newest = state.conversation.newest else { return .none }
                return go(to: newest, in: &state)

            case .core(.branchTapped(let id, let offset)):
                let branches = state.conversation.branches(at: id)
                guard let index = branches.firstIndex(where: { $0.id == id }),
                    branches.indices.contains(index + offset)
                else { return .none }
                return go(to: state.conversation.newest(through: branches[index + offset].id), in: &state)

            case .goTapped(let id):
                state.showsCheckpoints = false
                return go(to: id, in: &state)

            case .alert(.presented(.go(let id, let undo))):
                let entries = state.conversation.changedReplies(hiddenGoingTo: id).flatMap {
                    state.conversation.changes[$0] ?? []
                }
                state.core.rival = nil
                state.conversation.go(to: id)
                guard undo, !entries.isEmpty else { return .none }
                return .run { [store] send in
                    await send(.saved(Result { try await store.revert(entries: entries) }))
                }

            case .forkTapped(let id):
                state.showsCheckpoints = false
                guard state.canFork(at: id), let fork = state.conversation.fork(at: id, as: uuid(), now: now) else {
                    return .none
                }
                let from = state.conversation.id
                return .run { [attachments] send in
                    await attachments.copy(fork.messages.flatMap(\.attachments), from, fork.id)
                    await send(.forked(fork))
                }

            case .forked(let fork):
                return .send(.delegate(.fork(fork)))

            case .reloaded(let messageID, .success(let snapshot)):
                let workspace = snapshot.workspace
                state.reloading = false
                let known = Set(state.conversation.changes.values.joined()).union(state.conversation.added)
                let new = workspace.entries(inConversation: state.conversation.id).map(\.id)
                    .filter { !known.contains($0) }
                guard !new.isEmpty else { return .none }
                state.conversation.changes[messageID] = new
                if state.saves.isLatest(snapshot) { state.workspace = workspace }
                check(&state)
                return .send(.delegate(.saved(snapshot)))

            case .reloaded(_, .failure):
                // The assistant's changes are saved; the screens catch up on
                // the next update.
                state.reloading = false
                return .none

            case .undoTapped(let messageID):
                guard let entries = state.conversation.changes[messageID] else { return .none }
                return .run { [store] send in
                    await send(.saved(Result { try await store.revert(entries: entries) }))
                }

            case .applyTapped(let id):
                guard state.applying == nil, let proposal = state.proposal(of: id), proposal.decision == nil else {
                    return .none
                }
                state.applying = id
                let conversation = state.conversation.id
                return .run { [store, now, timeZone] send in
                    await send(
                        .applied(
                            messageID: id,
                            Result {
                                let workspace = try await store.load()
                                let made = try await Proposal.make(
                                    proposal.actions, on: workspace, now: now, timeZone: timeZone,
                                    conversation: conversation)
                                return try await store.save(made.operation)
                            }))
                }

            case .applied(let id, .success(let saved)):
                state.applying = nil
                if state.saves.isLatest(saved) { state.workspace = saved.workspace }
                state.decide(id, .applied)
                if let entry = saved.change.entry { state.conversation.changes[id, default: []].append(entry) }
                check(&state)
                return .send(.delegate(.saved(saved.snapshot)))

            case .applied(_, .failure(let error)):
                state.applying = nil
                state.alert = AlertState {
                    TextState("Couldn't apply")
                } actions: {
                    ButtonState(role: .cancel) { TextState("OK") }
                } message: {
                    TextState(
                        "\(String(describing: error)) Nothing changed. Edit the changes, or ask the assistant "
                            + "again for what you want now.")
                }
                return .none

            case .declineTapped(let id):
                guard state.proposal(of: id)?.decision == nil else { return .none }
                state.decide(id, .declined)
                return .none

            case .editProposalTapped(let id):
                guard let proposal = state.proposal(of: id), proposal.decision == nil else { return .none }
                state.editingProposal = ProposalEdit(id: id, actions: proposal.actions)
                return .none

            case .editProposalDone:
                guard let edit = state.editingProposal, state.applying == nil else { return .none }
                let actions = edit.chosen
                guard !actions.isEmpty else {
                    // Every change left out: nothing to apply.
                    state.editingProposal = nil
                    state.decide(edit.id, .declined)
                    return .none
                }
                state.applying = edit.id
                let conversation = state.conversation.id
                return .run { [store, now, timeZone] send in
                    await send(
                        .reproposed(
                            messageID: edit.id,
                            Result {
                                let workspace = try await store.load()
                                return try await Proposal.make(
                                    actions, on: workspace, now: now, timeZone: timeZone, conversation: conversation
                                ).proposal
                            }))
                }

            case .reproposed(let id, .success(let proposal)):
                state.applying = nil
                state.editingProposal = nil
                state.setProposal(proposal, of: id)
                return .none

            case .reproposed(_, .failure(let error)):
                state.applying = nil
                state.editingProposal?.problem = String(describing: error)
                return .none

            case .discardDraftTapped:
                guard state.canChangeDraft else { return .none }
                state.discardedDraft = state.conversation.draft
                state.conversation.draft = nil
                state.problem = nil
                state.editsDraft = false
                return .none

            case .undoDiscardTapped:
                guard state.conversation.draft == nil, let draft = state.discardedDraft else { return .none }
                state.conversation.draft = draft
                state.discardedDraft = nil
                check(&state)
                return .none

            case .core(.binding(let binding)) where binding.keyPath == \Core.State.input:
                let effect = core(.binding(binding), &state)
                if !state.warmed, !state.input.isEmpty, state.availability == .available {
                    state.warmed = true
                    assistant.prewarm(state.conversation.context.model)
                }
                // "/add " turns into the skill's name at the start of the
                // field, and "/compact " runs.
                if let typed = takeSlash(in: &state, complete: false) {
                    return .merge(effect, run(typed.command, details: typed.details, in: &state))
                }
                return effect

            case .binding(\.conversation.draft), .binding(\.conversation):
                // The editor's fields change the conversation they're in.
                // Removing the last row discards the draft.
                if state.conversation.draft?.isEmpty == true {
                    state.conversation.draft = nil
                    state.editsDraft = false
                }
                check(&state)
                return .none

            case .addTapped:
                guard state.canAdd, let plan = state.conversation.draft else { return .none }
                let conversation = state.conversation.id
                return .run { [store, timeZone] send in
                    await send(
                        .saved(
                            Result {
                                try await store.save { workspace, context throws(LifrError) in
                                    // The records may have changed since the draft was made.
                                    var change = try workspace.apply(
                                        plan.repaired(for: workspace), timeZone: timeZone, in: context)
                                    change.conversation = conversation
                                    return change
                                }
                            }))
                }

            case .saved(.success(let saved)):
                let isLatest = state.saves.isLatest(saved)
                if isLatest {
                    state.workspace = saved.workspace
                }
                if !saved.change.reverts.isEmpty {
                    let reverted = Set(saved.change.reverts)
                    for (message, entries) in state.conversation.changes where reverted.isSuperset(of: entries) {
                        state.conversation.changes[message] = nil
                    }
                    if state.conversation.draft == nil,
                        let entry = saved.change.reverts.first(where: { state.addedDrafts[$0] != nil })
                    {
                        state.conversation.draft = state.addedDrafts.removeValue(forKey: entry)
                    }
                } else {
                    // The plan was added.
                    if let entry = saved.change.entry, let draft = state.conversation.draft {
                        state.addedDrafts[entry] = draft
                    }
                    state.conversation.draft = nil
                    state.editsDraft = false
                    state.problem = nil
                    if let entry = saved.change.entry {
                        state.conversation.added.append(entry)
                        // The reply that drafted it made it, for going back
                        // past that reply to undo.
                        if let reply = state.conversation.messages.last(where: { $0.role == .assistant }) {
                            state.conversation.changes[reply.id, default: []].append(entry)
                        }
                    }
                }
                check(&state)
                return isLatest ? .send(.delegate(.saved(saved.snapshot))) : .none

            case .saved(.failure(let error)):
                state.problem = String(describing: error)
                // The draft says what's wrong with its plan.
                if case .plan? = error as? LifrError { return .none }
                state.alert = AlertState {
                    TextState("Couldn't save")
                } message: {
                    TextState(
                        "\(error) Nothing changed. Send a message and the assistant will fix the draft.")
                }
                return .none

            case .core(.delegate):
                // Lifr keeps the conversation as it changes.
                return .none

            case .core(let action):
                return core(action, &state)

            case .binding, .alert, .manageSkills, .model, .delegate:
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
        .ifLet(\.$manageSkills, action: \.manageSkills) { SkillsFeature() }
        .ifLet(\.$model, action: \.model) { ChatModelFeature() }
        Reduce { state, _ in
            state.sync()
            return .none
        }
    }

    /// Runs `action` in the shared chat, which keeps its photos with the
    /// conversation's other attachments.
    /// Stops the microphone and attaches what it heard.
    private func stopRecording(_ state: inout State) -> Effect<Action> {
        state.recording = false
        state.recordingLive = false
        state.core.attaching = true
        let conversation = state.conversation.id
        return .merge(
            .cancel(id: CancelID.recording),
            .run { [attachments, recorder] send in
                await send(
                    .recorded(
                        Result {
                            let recording = try await recorder.stop()
                            return try await attachments.save(recording.data, .audio, recording.seconds, conversation)
                        }))
            })
    }

    /// Stops the microphone, or keeps it from starting, and keeps nothing.
    private func cancelRecording(_ state: inout State) -> Effect<Action> {
        state.recording = false
        state.recordingLive = false
        return .merge(.cancel(id: CancelID.recording), .run { [recorder] _ in await recorder.cancel() })
    }

    private func core(_ action: Core.Action, _ state: inout State) -> Effect<Action> {
        Scope(state: \.core, action: \.core) {
            Core(replier: LifrReplier()).dependency(\.chatPhotos, .attachments(attachments))
        }
        .reduce(into: &state, action: .core(action))
    }

    /// Sends what's in the field, with the skill picked and the
    /// recordings, letting Lifr pick a skill when none is.
    private func send(in state: inout State) -> Effect<Action> {
        state.skill = state.picked?.id
        let picks = state.conversation.context.picksSkills ?? state.picksSkills
        state.core.host.pick = state.skill == nil && picks ? pickable(in: state) : nil
        let effect = core(.sendTapped, &state)
        state.core.host.pick = nil
        if state.core.reply != nil {
            state.skill = nil
            state.recordings = []
            state.discardedDraft = nil
        }
        return effect
    }

    /// Keeps what the reply `id` drafted and changed, then names the
    /// conversation and compacts it when Lifr does.
    private func finish(_ extra: ReplyChanges?, of id: UUID, in state: inout State) -> Effect<Action> {
        state.stopped = false
        state.reloading = true
        state.leftOut = extra?.leftOut ?? 0
        if let plan = extra?.plan { draft(plan, in: &state) }
        var effect: Effect<Action> = .run { [store] send in
            await send(.reloaded(messageID: id, Result { try await store.snapshot() }))
        }
        if state.compactsConversations, state.offersCompact { effect = .merge(effect, compact(&state)) }
        guard state.namesConversations, !state.conversation.isNamedByHand, state.conversation.name == nil
        else { return effect }
        state.naming = true
        let messages = state.conversation.messages
        return .merge(
            effect,
            .run { [assistant] send in
                await send(.named(Result { try await assistant.suggestTitle(messages, nil) }))
            })
    }

    /// Puts the reply's `plan` on the draft card.
    private func draft(_ plan: Plan, in state: inout State) {
        // A model often repeats what this chat already added, and makes
        // small mistakes Lifr can put right. A plan of only what's there
        // already leaves the draft as it was, as no plan does; an empty
        // plan, such as after every task was removed, clears it.
        let new = plan.leavingOut(state.workspace).repaired(for: state.workspace)
        if !new.isEmpty || plan.isEmpty { state.conversation.draft = new.isEmpty ? nil : new }
        check(&state)
    }

    /// Summarizes the messages before the last two, after the newest
    /// summary, which the new one takes in.
    private func compact(_ state: inout State) -> Effect<Action> {
        let messages = Array(state.conversation.compactable)
        guard let through = messages.last?.id else { return .none }
        state.compacting = true
        let earlier = state.conversation.summary?.summary.text
        let choice = state.conversation.context.model
        return .run { [assistant] send in
            await send(
                .compacted(through: through, Result { try await assistant.compact(messages, earlier, choice) }))
        }
    }

    private static func attachAlert(_ error: any Error) -> AlertState<Action.Alert> {
        AlertState {
            TextState("Couldn't attach that")
        } actions: {
            ButtonState(role: .cancel) { TextState("OK") }
        } message: {
            TextState(String(describing: error))
        }
    }

    /// Goes to the message `id`, first asking whether to undo the task
    /// changes of the replies that hides. Going forward again doesn't make
    /// undone changes again.
    private func go(to id: UUID, in state: inout State) -> Effect<Action> {
        guard state.canGo else { return .none }
        state.leftOut = 0
        let changed = state.conversation.changedReplies(hiddenGoingTo: id)
        guard !changed.isEmpty else {
            state.core.rival = nil
            state.conversation.go(to: id)
            return .none
        }
        state.alert = notice(changed, in: state, undo: .go(to: id, undo: true), keep: .go(to: id, undo: false))
        return .none
    }

    /// Asks whether to undo the task changes of the replies `changed`,
    /// which what the person asked for hides.
    private func notice(
        _ changed: [UUID], in state: State, undo: Action.Alert, keep: Action.Alert
    ) -> AlertState<Action.Alert> {
        let count = changed.reduce(0) { $0 + (state.conversation.changes[$1]?.count ?? 0) }
        let changes = String(AttributedString(localized: "^[\(count) change](inflect: true)").characters)
        return AlertState {
            TextState("Also undo the \(changes) made after this point?")
        } actions: {
            ButtonState(role: .destructive, action: undo) { TextState("Undo Changes") }
            ButtonState(action: keep) { TextState("Keep Changes") }
            ButtonState(role: .cancel) { TextState("Cancel") }
        } message: {
            TextState(
                "The replies after it are kept to go forward to. Going forward doesn't make undone changes again.")
        }
    }

    /// Undoes the changes of the replies `changed`, then sends `next`. When
    /// undoing fails, it says so and sends nothing, for the person to
    /// choose again.
    private func undoing(_ changed: [UUID], in state: State, then next: @escaping @Sendable () -> Action)
        -> Effect<Action>
    {
        let entries = changed.flatMap { state.conversation.changes[$0] ?? [] }
        guard !entries.isEmpty else { return .send(next()) }
        return .run { [store] send in
            let result = await Result { try await store.revert(entries: entries) }
            await send(.saved(result))
            if case .success = result { await send(next()) }
        }
    }

    /// Takes "/name" and the space or new line after it from the start of
    /// the field, while no skill is picked: a skill becomes the one picked,
    /// with what follows left as its details; a command is returned to run,
    /// with what follows. A name not complete yet, or that's neither, stays
    /// as text.
    private func takeSlash(in state: inout State, complete: Bool) -> (command: ChatCommand, details: String)? {
        guard state.picked == nil, let typed = TypedSlash(state.input, complete: complete),
            let choice = state.slash(named: typed.name)
        else { return nil }
        state.input = typed.details
        switch choice {
        case .skill(let skill):
            state.skill = skill.id
            return nil
        case .command(let command):
            return (command, typed.details)
        }
    }

    /// Runs `command`, typed with `details` after it: the words to find,
    /// for /find. The others leave them in the field.
    private func run(_ command: ChatCommand, details: String, in state: inout State) -> Effect<Action> {
        switch command {
        case .compact: return .send(.compactTapped)
        case .find:
            state.input = ""
            state.core.finds = true
            state.core.findQuery = details.trimmingCharacters(in: .whitespacesAndNewlines)
            state.core.findPosition = max(state.core.findMatches.count - 1, 0)
            return .none
        case .back: return .send(.backTapped)
        case .forward: return .send(.forwardTapped)
        case .checkpoints:
            state.showsCheckpoints = true
            return .none
        case .rename: return .send(.delegate(.rename))
        case .model: return .send(.core(.modelTapped))
        case .skills: return .send(.manageSkillsTapped)
        case .new: return .send(.delegate(.new))
        }
    }

    /// The skills Lifr may pick for a message, and the one to keep when it
    /// picks none while a plan is being drafted: the last the person's
    /// messages used, so answers about the plan still change it.
    private func pickable(in state: State) -> SkillPick {
        let candidates = Skill.pickable(
            state.skills, chat: state.conversation.context.picksSkills, app: state.picksSkills)
        let drafting = !(state.conversation.draft?.isEmpty ?? true)
        let last = state.conversation.messages.last { $0.role == .person && $0.skill != nil }?.skill
        let kept = drafting ? last.flatMap { id in state.skills.first { $0.id == id && $0.isOn } } : nil
        return SkillPick(candidates: candidates, kept: kept)
    }

    /// Checks the draft against the records as they are now.
    private func check(_ state: inout State) {
        state.problem = state.conversation.draft.flatMap { plan in
            state.workspace.problem(with: plan, timeZone: timeZone, now: now)?.description
        }
    }
}
