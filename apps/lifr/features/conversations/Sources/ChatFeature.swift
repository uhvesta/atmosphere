import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrCore
import LifrModelsClient
import LifrSkills
import LifrStoreClient
import NaturalLanguage

/// A chat with the assistant: it answers questions, proposes changes to
/// tasks and Projects when asked, which the person applies, edits or
/// declines (each reply applied has Undo), and drafts plans on a card the
/// person can edit and then add. A reply keeps going when the person
/// leaves the chat or Lifr; one the system turns away, as Apple's model
/// does in the background, is sent again on `resume`.
@Reducer
package struct ChatFeature {
    @ObservableState
    package struct State: Equatable, Identifiable {
        package var conversation: Conversation
        package var workspace: Workspace
        /// The latest save shown, so an earlier one's late reply is ignored.
        package var saves = SaveOrder()
        package var input = ""
        package var availability = AssistantAvailability.available
        /// The reply as it's written, until it's complete.
        package var replying: String?
        /// What the model has thought so far for the reply being written.
        package var replyingThinking: ModelThinking?
        /// Whether the thinking of the reply being written is open.
        package var showsReplyThinking = false
        /// The replies whose thinking is open.
        package var shownThinking: Set<UUID> = []
        /// The request of the reply being written, kept until it's complete.
        package var waiting: AssistantRequest?
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
        /// Why the draft can't be added as it is now, checked on every
        /// change, and sent with the next message.
        package var problem: String?
        /// The day the draft's dates are read from, such as "saturday".
        package var today: CalendarDate?
        @Presents package var alert: AlertState<Action.Alert>?
        /// Why the last reply couldn't be written, shown under the message
        /// it's to.
        package var failure: Failure?
        /// The person's message being written again: sending puts the new
        /// one in its place, as a branch beside it.
        package var editing: UUID?
        /// Whether the bar to find words in the messages shown is up.
        package var finds = false
        /// What's being found in the messages shown.
        package var findQuery = ""
        /// Which of `findMatches` is current, counting from the oldest.
        package var findPosition = 0
        /// Every skill, on or off. The built-ins until the person's load.
        package var skills = Skill.builtIn
        /// The ID of the skill picked for the next message, shown as its
        /// "/name" at the start of the field.
        package var skill: String?
        @Presents package var manageSkills: SkillsFeature.State?
        /// What the conversation runs on, being picked.
        @Presents package var model: ChatModelFeature.State?
        /// Apple's model and the downloaded ones, and the assistant's in
        /// Settings, once they're looked up: for the composer's pill.
        package var models: [RunnableModel] = []
        package var standard: ModelID?
        /// How much of the model's window the last reply took, since the
        /// chat opened or its model changed.
        package var context: ContextUse?
        /// Whether the draft's editor is open.
        package var editsDraft = false
        /// The proposal being edited, before it's checked again.
        package var editingProposal: ProposalEdit?
        /// The reply whose proposal is being applied or checked again.
        package var applying: UUID?
        /// What the assistant's model reads besides text: the composer only
        /// offers to attach what it can read.
        package var inputs: Set<ModelInput> = []
        /// Photos and recordings to send with the next message.
        package var pending: [ChatAttachment] = []
        /// Whether a photo or recording is being kept for the next message.
        package var attaching = false
        /// Whether the microphone is recording for the next message.
        package var recording = false
        /// Attachments of this conversation whose files aren't on this
        /// device, such as those sent from another one.
        package var elsewhere: Set<UUID> = []
        /// Whether the model was asked to load, once the person started
        /// writing. Opening a chat doesn't: Lifr reopens the last one at
        /// launch, and a model takes gigabytes.
        package var warmed = false
        /// Whether the log of every point to go back or forward to is open.
        package var showsCheckpoints = false
        /// The reply being read aloud.
        package var speaking: UUID?

        package init(conversation: Conversation, workspace: Workspace) {
            self.conversation = conversation
            self.workspace = workspace
            forgetUndone()
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
        /// offer Undo again.
        private mutating func forgetUndone() {
            guard !conversation.changes.isEmpty else { return }
            let undone = workspace.undoneEntries
            for (message, entries) in conversation.changes where undone.isSuperset(of: entries) {
                conversation.changes[message] = nil
            }
        }

        package var id: UUID { conversation.id }

        /// Whether a reply is being written, waits to be sent again, is
        /// being finished, or failed: the chat is kept until it isn't.
        package var isWorking: Bool {
            replying != nil || waiting != nil || reloading || naming || compacting || alert != nil
                || failure != nil
        }

        /// Whether older messages can be summarized now: there are some
        /// after the newest summary, and no reply is being written.
        package var canCompact: Bool {
            availability == .available && replying == nil && waiting == nil && !compacting
                && !conversation.compactable.isEmpty
        }

        /// Whether to offer Compact: the last reply was written without
        /// some of the conversation.
        package var offersCompact: Bool { leftOut > 0 && canCompact }

        package var canAdd: Bool { conversation.draft != nil && problem == nil }

        /// Whether the conversation can go back, forward or to another
        /// branch: not while a reply is written or its changes are read.
        package var canGo: Bool {
            replying == nil && waiting == nil && !reloading && !compacting && !conversation.messages.isEmpty
        }

        /// Whether the conversation can be forked at the message `id`: also
        /// while a reply is written, from before the message it answers.
        /// The messages shown that have every word being found, oldest
        /// first.
        package var findMatches: [UUID] {
            let query = ConversationQuery(words: findQuery.split(whereSeparator: \.isWhitespace).map(String.init))
            return conversation.messages.filter { query.matches($0.text) }.map(\.id)
        }

        /// The match shown now, when there's one.
        package var currentMatch: UUID? {
            let matches = findMatches
            return matches.indices.contains(findPosition) ? matches[findPosition] : nil
        }

        package func canFork(at id: UUID) -> Bool {
            guard !conversation.messages.isEmpty else { return false }
            return canGo || id != conversation.messages.last?.id
        }

        /// Whether the reply to `message`, or `message` itself for a reply,
        /// can be written again: any reply, or the last message when its
        /// reply failed in a way sending it again can fix, or never came.
        package func canRetry(_ message: ChatMessage) -> Bool {
            guard canGo, availability == .available else { return false }
            guard message.role == .person else { return true }
            return message.id == conversation.messages.last?.id && failure?.retries ?? true
        }

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

        /// The model the next message runs on, once it's looked up.
        package var current: RunnableModel? {
            let id = conversation.context.model.model ?? standard
            return models.first { $0.id == id }
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

        /// Whether a reply is being written, which Stop ends.
        package var canStop: Bool { waiting != nil }

        package var canSend: Bool {
            availability == .available && replying == nil && waiting == nil && !attaching && !recording && !compacting
                && (picked != nil || !pending.isEmpty
                    || !input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case findTapped
        /// Goes to the match `by` after the current one, or before it when
        /// negative, coming round at the ends.
        case findMoved(by: Int)
        case findClosed
        case skillsLoaded([Skill])
        case pickSkillTapped(String)
        /// A command picked from the / list, which runs now.
        case pickCommandTapped(ChatCommand)
        case clearSkillTapped
        case manageSkillsTapped
        case manageSkills(PresentationAction<SkillsFeature.Action>)
        /// Opens what the conversation runs on, to pick another model,
        /// thinking or window for it.
        case modelTapped
        case model(PresentationAction<ChatModelFeature.Action>)
        case modelsLoaded([RunnableModel], standard: ModelID)
        case sendTapped
        /// Stops the reply being written, keeping what it has written.
        case stopTapped
        /// A suggestion in an empty chat: sent as the person's message.
        case suggestionTapped(String)
        case editDraftTapped
        /// A photo the person picked, in any format the system reads.
        case photoPicked(Data)
        /// Starts recording, or stops and attaches the recording.
        case recordTapped
        case cancelRecordingTapped
        case recordingStarted(Result<Void, any Error>)
        case attached(Result<ChatAttachment, any Error>)
        case removeAttachmentTapped(UUID)
        /// Lifr is back on screen: send a stopped reply's request again.
        case resume
        /// Lifr picked the skill `id` for the message `messageID`, sent
        /// without one.
        case skillPicked(messageID: UUID, id: String)
        case replied(AssistantReply)
        case replyFailed(any Error)
        /// The thinking of the reply `messageID` was opened or closed, or of
        /// the reply being written when nil.
        case thinkingToggled(messageID: UUID?)
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
        case addTapped
        case saved(Result<Saved, any Error>)
        /// Goes to the point before the last one shown.
        case backTapped
        /// Goes to the point after the last one shown, the way it last went.
        case forwardTapped
        /// Goes to the end of the latest branch from here.
        case latestTapped
        /// Shows the branch `by` places along from the message's, such as
        /// -1 for the one written before it.
        case branchTapped(messageID: UUID, by: Int)
        /// A point in the log, or a message, to go to.
        case goTapped(UUID)
        /// Makes a new conversation of the messages up to this one.
        case forkTapped(messageID: UUID)
        case forked(Conversation)
        /// Puts one of the person's messages in the field, to send again in
        /// its place.
        case editTapped(messageID: UUID)
        case cancelEditTapped
        /// Writes the reply to this message, or this reply, again.
        case retryTapped(messageID: UUID)
        /// Reads the reply aloud, or stops reading it.
        case speakTapped(messageID: UUID)
        /// The reply finished being read aloud, or why it couldn't be.
        case spoke(messageID: UUID, SpeechError?)
        /// Sends the message being edited, once the person said whether to
        /// undo the changes of the replies that hides.
        case sendConfirmed
        /// Writes the reply to the person's message `id` again, once the
        /// person said whether to undo the changes of the replies that hides.
        case retryConfirmed(UUID)
        case alert(PresentationAction<Alert>)
        case delegate(Delegate)

        package enum Alert: Equatable {
            /// Goes to the message, undoing the changes of the replies
            /// that hides or keeping them.
            case go(to: UUID, undo: Bool)
            /// Sends the message being edited in place of the old one.
            case resend(undo: Bool)
            /// Writes the reply to the person's message again.
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

    /// Why a reply couldn't be written.
    package struct Failure: Equatable, Sendable {
        package var reason: String
        /// Whether sending it again can fix it: not for a request that's
        /// too long or declined.
        package var retries: Bool
    }

    private enum CancelID { case reply, recording, speech }

    /// How long the assistant may write nothing before Lifr stops waiting:
    /// it gives up between one and two of these after the last progress.
    package static let stallLimit = Duration.seconds(60)

    @Dependency(\.assistant) private var assistant
    @Dependency(\.store) private var store
    @Dependency(\.skills) private var skillsClient
    @Dependency(\.models) private var library
    @Dependency(\.guidance) private var guidance
    @Dependency(\.attachments) private var attachments
    @Dependency(\.recorder) private var recorder
    @Dependency(\.speech) private var speech
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone
    @Dependency(\.uuid) private var uuid
    @Dependency(\.backgroundTime) private var backgroundTime
    @Dependency(\.continuousClock) private var clock

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .task:
                state.availability = assistant.availability(state.conversation.context.model)
                state.today = CalendarDate(now, in: timeZone)
                state.inputs = assistant.inputs(state.conversation.context.model)
                let conversation = state.conversation.id
                state.elsewhere = Set(
                    state.conversation.messages.flatMap(\.attachments)
                        .filter { attachments.file($0, conversation) == nil }.map(\.id))
                return .run { [skillsClient, library] send in
                    await send(.modelsLoaded(await library.runnable(), standard: library.selection(.assistant)))
                    // Without the person's skills, the built-ins still work.
                    if let skills = try? await skillsClient.load() { await send(.skillsLoaded(skills)) }
                }

            case .modelsLoaded(let models, let standard):
                state.models = models
                state.standard = standard
                return .none

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

            case .modelTapped:
                state.model = ChatModelFeature.State(
                    choice: state.conversation.context.model, picksSkills: state.conversation.context.picksSkills)
                return .none

            case .model(.presented):
                if let sheet = state.model, !sheet.models.isEmpty {
                    state.models = sheet.models
                    state.standard = sheet.standard
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
                    state.context = nil
                }
                state.conversation.context.model = choice
                state.availability = assistant.availability(choice)
                state.inputs = assistant.inputs(choice)
                // The new model loads once the person writes.
                state.warmed = false
                return .none

            case .manageSkills(.dismiss):
                return .run { [skillsClient] send in
                    if let skills = try? await skillsClient.load() { await send(.skillsLoaded(skills)) }
                }

            case .sendTapped:
                // "/add buy milk" picks the skill and sends "buy milk".
                if let typed = takeSlash(in: &state, complete: true) {
                    return run(typed.command, details: typed.details, in: &state)
                }
                guard state.canSend else { return .none }
                if let editing = state.editing {
                    let changed = state.conversation.changedReplies(hiddenGoingBefore: editing)
                    guard changed.isEmpty else {
                        state.alert = undoAlert(
                            changed, in: state, undo: .resend(undo: true), keep: .resend(undo: false))
                        return .none
                    }
                }
                return send(in: &state)

            case .sendConfirmed:
                guard state.canSend else { return .none }
                return send(in: &state)

            case .editTapped(let id):
                guard state.canGo,
                    let message = state.conversation.messages.first(where: { $0.id == id && $0.role == .person })
                else { return .none }
                state.editing = id
                state.input = message.text
                state.skill = message.skill
                return .none

            case .findTapped:
                state.finds = true
                return .none

            case .findMoved(let step):
                let count = state.findMatches.count
                guard count > 0 else { return .none }
                state.findPosition = ((state.findPosition + step) % count + count) % count
                return .none

            case .findClosed:
                state.finds = false
                state.findQuery = ""
                state.findPosition = 0
                return .none

            case .binding(\.findQuery):
                // The newest match first, nearest where the chat ends.
                state.findPosition = max(state.findMatches.count - 1, 0)
                return .none

            case .cancelEditTapped:
                state.editing = nil
                state.input = ""
                state.skill = nil
                return .none

            case .speakTapped(let id):
                if state.speaking == id {
                    state.speaking = nil
                    return .merge(.cancel(id: CancelID.speech), .run { _ in await speech.stop() })
                }
                guard let message = state.conversation.messages.first(where: { $0.id == id && $0.role == .assistant })
                else { return .none }
                let text = Self.spoken(message.text)
                guard !text.isEmpty else { return .none }
                state.speaking = id
                return .run { send in
                    do throws(SpeechError) {
                        try await speech.speak(text, Self.language(of: text))
                        await send(.spoke(messageID: id, nil))
                    } catch {
                        await send(.spoke(messageID: id, error))
                    }
                }
                .cancellable(id: CancelID.speech, cancelInFlight: true)

            case .spoke(let id, let problem):
                if state.speaking == id { state.speaking = nil }
                if let problem {
                    state.alert = AlertState {
                        TextState("Couldn't read it aloud")
                    } actions: {
                        ButtonState(role: .cancel) { TextState("OK") }
                    } message: {
                        TextState(problem.description)
                    }
                }
                return .none

            case .retryTapped(let id):
                let messages = state.conversation.messages
                guard let index = messages.firstIndex(where: { $0.id == id }), state.canRetry(messages[index]),
                    let asked = messages[...index].last(where: { $0.role == .person })
                else { return .none }
                let changed = state.conversation.changedReplies(hiddenGoingTo: asked.id)
                guard changed.isEmpty else {
                    state.alert = undoAlert(
                        changed, in: state, undo: .retry(asked.id, undo: true), keep: .retry(asked.id, undo: false))
                    return .none
                }
                return .send(.retryConfirmed(asked.id))

            case .retryConfirmed(let id):
                guard state.canGo else { return .none }
                state.conversation.go(to: id)
                guard state.conversation.messages.last?.id == id else { return .none }
                state.failure = nil
                return reply(request(in: state), in: &state)

            case .alert(.presented(.resend(let undo))):
                guard let editing = state.editing else { return .none }
                let changed = undo ? state.conversation.changedReplies(hiddenGoingBefore: editing) : []
                return undoing(changed, in: state) { .sendConfirmed }

            case .alert(.presented(.retry(let id, let undo))):
                let changed = undo ? state.conversation.changedReplies(hiddenGoingTo: id) : []
                return undoing(changed, in: state) { .retryConfirmed(id) }

            case .suggestionTapped(let text):
                guard state.conversation.messages.isEmpty else { return .none }
                state.input = text
                return .send(.sendTapped)

            case .editDraftTapped:
                guard state.conversation.draft != nil else { return .none }
                state.editsDraft = true
                return .none

            case .photoPicked(let data):
                guard state.inputs.contains(.image), !state.attaching else { return .none }
                state.attaching = true
                let conversation = state.conversation.id
                return .run { [attachments] send in
                    await send(.attached(Result { try await attachments.save(data, .image, nil, conversation) }))
                }

            case .recordTapped where state.recording:
                state.recording = false
                state.attaching = true
                let conversation = state.conversation.id
                return .merge(
                    .cancel(id: CancelID.recording),
                    .run { [attachments, recorder] send in
                        await send(
                            .attached(
                                Result {
                                    let recording = try await recorder.stop()
                                    return try await attachments.save(
                                        recording.data, .audio, recording.seconds, conversation)
                                }))
                    })

            case .recordTapped:
                guard state.inputs.contains(.audio), !state.attaching else { return .none }
                state.recording = true
                return .run { [recorder] send in
                    await send(.recordingStarted(Result { try await recorder.start() }))
                }
                .cancellable(id: CancelID.recording)

            case .recordingStarted(.success):
                return .none

            case .recordingStarted(.failure(let error)):
                state.recording = false
                state.alert = Self.attachAlert(error)
                return .none

            case .cancelRecordingTapped:
                guard state.recording else { return .none }
                state.recording = false
                return .merge(.cancel(id: CancelID.recording), .run { [recorder] _ in await recorder.cancel() })

            case .attached(.success(let attachment)):
                state.attaching = false
                state.pending.append(attachment)
                return .none

            case .attached(.failure(let error)):
                state.attaching = false
                state.alert = Self.attachAlert(error)
                return .none

            case .removeAttachmentTapped(let id):
                guard let attachment = state.pending.first(where: { $0.id == id }) else { return .none }
                state.pending.removeAll { $0.id == id }
                let conversation = state.conversation.id
                return .run { [attachments] _ in await attachments.remove(attachment, conversation) }

            case .stopTapped:
                guard state.canStop else { return .none }
                let written = (state.replying ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
                let thinking = state.replyingThinking
                state.replying = nil
                state.replyingThinking = nil
                state.waiting = nil
                state.stopped = false
                // What it was asking to change is left out with the rest:
                // nothing's saved before the person applies it.
                if !written.isEmpty || thinking != nil {
                    let id = uuid()
                    state.conversation.append(
                        ChatMessage(id: id, role: .assistant, text: written, at: now, thinking: thinking))
                    if state.showsReplyThinking, thinking != nil { state.shownThinking.insert(id) }
                }
                state.showsReplyThinking = false
                return .cancel(id: CancelID.reply)

            case .resume:
                // A reply still being written carries on by itself.
                guard state.stopped, let request = state.waiting else { return .none }
                return reply(request, in: &state)

            case .skillPicked(let messageID, let id):
                guard let index = state.conversation.messages.firstIndex(where: { $0.id == messageID }) else {
                    return .none
                }
                state.conversation.messages[index].skill = id
                let skill = state.skills.first { $0.id == id }
                if state.waiting?.messages.last?.id == messageID { state.waiting?.skill = skill }
                return .none

            case .replied(let reply):
                guard reply.isComplete else {
                    state.replying = reply.text
                    state.replyingThinking = reply.thinking?.shown
                    return .none
                }
                state.replying = nil
                state.replyingThinking = nil
                state.waiting = nil
                state.stopped = false
                state.reloading = true
                let id = uuid()
                let thinking = reply.thinking?.shown
                state.conversation.append(
                    ChatMessage(
                        id: id, role: .assistant, text: reply.text, at: now, thinking: thinking,
                        proposal: reply.proposal))
                // Thinking opened while it was written stays open.
                if state.showsReplyThinking, thinking != nil { state.shownThinking.insert(id) }
                state.showsReplyThinking = false
                state.leftOut = reply.leftOut
                if let context = reply.context { state.context = context }
                if let plan = reply.plan {
                    // A model often repeats what this chat already added,
                    // and makes small mistakes Lifr can put right. A plan
                    // of only what's there already leaves the draft as it
                    // was, as no plan does; an empty plan, such as after
                    // every task was removed, clears it.
                    let new = plan.leavingOut(state.workspace).repaired(for: state.workspace)
                    if !new.isEmpty || plan.isEmpty { state.conversation.draft = new.isEmpty ? nil : new }
                    check(&state)
                }
                var reload: Effect<Action> = .run { [store] send in
                    await send(.reloaded(messageID: id, Result { try await store.snapshot() }))
                }
                if state.compactsConversations, state.offersCompact { reload = .merge(reload, compact(&state)) }
                guard state.namesConversations, !state.conversation.isNamedByHand, state.conversation.name == nil
                else { return reload }
                state.naming = true
                let messages = state.conversation.messages
                return .merge(
                    reload,
                    .run { [assistant] send in
                        await send(.named(Result { try await assistant.suggestTitle(messages, nil) }))
                    })

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

            case .replyFailed(let error as AssistantError) where error == .busy:
                // Keep what's written so far, and send again on `resume`.
                state.stopped = true
                return .none

            case .thinkingToggled(let id?):
                if state.shownThinking.remove(id) == nil { state.shownThinking.insert(id) }
                return .none

            case .thinkingToggled(nil):
                state.showsReplyThinking.toggle()
                return .none

            case .replyFailed(let error):
                state.replying = nil
                state.replyingThinking = nil
                state.showsReplyThinking = false
                // Sending again can't help a request that's too long or
                // declined, and could make saved changes twice.
                let retries =
                    (error as? AssistantError).map {
                        $0 == .unreadable || $0 == .timedOut || $0.isFailure
                    } ?? true
                state.failure = Failure(reason: String(describing: error), retries: retries)
                state.waiting = nil
                state.stopped = false
                return .none

            case .backTapped:
                guard let back = state.conversation.back else { return .none }
                return go(to: back, in: &state)

            case .forwardTapped:
                guard let forward = state.conversation.forward else { return .none }
                return go(to: forward, in: &state)

            case .latestTapped:
                guard let newest = state.conversation.newest else { return .none }
                return go(to: newest, in: &state)

            case .branchTapped(let id, let offset):
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
                state.conversation.draft = nil
                state.problem = nil
                state.editsDraft = false
                return .none

            case .binding(\.input):
                if !state.warmed, !state.input.isEmpty, state.availability == .available {
                    state.warmed = true
                    assistant.prewarm(state.conversation.context.model)
                }
                // "/add " turns into the skill's name at the start of the
                // field, and "/compact " runs.
                if let typed = takeSlash(in: &state, complete: false) {
                    return run(typed.command, details: typed.details, in: &state)
                }
                return .none

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
                                    var change = try workspace.apply(plan, timeZone: timeZone, in: context)
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
                } else {
                    // The plan was added.
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
                state.alert = AlertState {
                    TextState("Couldn't save")
                } message: {
                    TextState(
                        "\(error) Nothing changed. Send a message and the assistant will fix the draft.")
                }
                return .none

            case .binding, .alert, .manageSkills, .model, .delegate:
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
        .ifLet(\.$manageSkills, action: \.manageSkills) { SkillsFeature() }
        .ifLet(\.$model, action: \.model) { ChatModelFeature() }
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

    /// Writes the reply to `request`, with background time so it keeps
    /// going when the person leaves Lifr. Until it's complete, `waiting`
    /// keeps the request for `resume`.
    private func reply(
        _ request: AssistantRequest, picking pick: SkillPick? = nil, for messageID: UUID? = nil, in state: inout State
    ) -> Effect<Action> {
        state.waiting = request
        state.stopped = false
        if state.replying == nil {
            state.replying = ""
            state.replyingThinking = nil
        }
        return .run { [assistant, backgroundTime, clock, guidance] send in
            var request = request
            // The default when the person's can't be read; the
            // Instructions screen says why.
            request.guidance = (try? await guidance.load().text) ?? Guidance.defaultText
            if let pick, let messageID {
                // A pick that fails leaves the message without a skill, to
                // answer as it is.
                let picked = try? await assistant.pickSkill(
                    request.messages, pick.candidates, pick.kept?.id, request.model)
                let id = picked ?? pick.kept?.id
                if let id, let skill = (pick.candidates + (pick.kept.map { [$0] } ?? [])).first(where: { $0.id == id })
                {
                    request.skill = skill
                    await send(.skillPicked(messageID: messageID, id: id))
                }
            }
            try await backgroundTime.run("Answering") { [request] progressed in
                try await Self.watch(
                    request, assistant: assistant, clock: clock, returns: backgroundTime.returns
                ) { reply in
                    progressed()
                    await send(.replied(reply))
                }
            }
        } catch: { error, send in
            await send(.replyFailed(error))
        }
        .cancellable(id: CancelID.reply, cancelInFlight: true)
    }

    /// Sends each reply to `request`, and stops with `.timedOut` when the
    /// assistant writes nothing for `stallLimit`. Time in the
    /// background doesn't count: a model there may be waiting for Lifr to
    /// be back on screen, so each return starts the wait again.
    private static func watch(
        _ request: AssistantRequest, assistant: AssistantClient, clock: any Clock<Duration>,
        returns: @escaping @Sendable () -> Int, _ each: @escaping @Sendable (AssistantReply) async -> Void
    ) async throws {
        let progress = LockIsolated(0)
        try await withThrowingTaskGroup(of: Void.self) { [request] group in
            group.addTask {
                for try await reply in assistant.reply(request) {
                    progress.withValue { $0 += 1 }
                    await each(reply)
                }
            }
            group.addTask {
                var seen = -1
                var back = -1
                while progress.value != seen || returns() != back {
                    seen = progress.value
                    back = returns()
                    try await clock.sleep(for: stallLimit)
                }
                throw AssistantError.timedOut
            }
            try await group.next()
            group.cancelAll()
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
            state.conversation.go(to: id)
            return .none
        }
        state.alert = undoAlert(changed, in: state, undo: .go(to: id, undo: true), keep: .go(to: id, undo: false))
        return .none
    }

    /// Asks whether to undo the task changes of the replies `changed`,
    /// which what the person asked for hides.
    private func undoAlert(
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
            state.finds = true
            state.findQuery = details.trimmingCharacters(in: .whitespacesAndNewlines)
            state.findPosition = max(state.findMatches.count - 1, 0)
            return .none
        case .back: return .send(.backTapped)
        case .forward: return .send(.forwardTapped)
        case .checkpoints:
            state.showsCheckpoints = true
            return .none
        case .rename: return .send(.delegate(.rename))
        case .model: return .send(.modelTapped)
        case .skills: return .send(.manageSkillsTapped)
        case .new: return .send(.delegate(.new))
        }
    }

    /// Sends what's in the field, in place of the message being edited if
    /// there is one.
    private func send(in state: inout State) -> Effect<Action> {
        let text = state.input.trimmingCharacters(in: .whitespacesAndNewlines)
        state.input = ""
        let skill = state.picked
        state.skill = nil
        var sent = state.pending
        state.pending = []
        if let editing = state.editing {
            // The new message keeps what was attached to the old one.
            let old = (state.conversation.messages + state.conversation.hidden).first { $0.id == editing }
            sent = (old?.attachments ?? []) + sent
            state.conversation.go(before: editing)
            state.editing = nil
        }
        state.failure = nil
        let id = uuid()
        state.conversation.append(
            ChatMessage(id: id, role: .person, text: text, at: now, skill: skill?.id, attachments: sent))
        guard skill == nil, state.conversation.context.picksSkills ?? state.picksSkills else {
            return reply(request(in: state), in: &state)
        }
        return reply(request(in: state), picking: pickable(in: state), for: id, in: &state)
    }

    /// The skills Lifr may pick for a message, and the one to keep when it
    /// picks none while a plan is being drafted: the last the person's
    /// messages used, so answers about the plan still change it.
    private func pickable(in state: State) -> SkillPick {
        let candidates = Skill.pickable(
            state.skills, chat: state.conversation.context.picksSkills, app: state.picksSkills)
        let drafting = !(state.conversation.draft?.isEmpty ?? true)
        let last = state.conversation.messages.dropLast().last { $0.role == .person && $0.skill != nil }?.skill
        let kept = drafting ? last.flatMap { id in state.skills.first { $0.id == id && $0.isOn } } : nil
        return SkillPick(candidates: candidates, kept: kept)
    }

    /// The request for a reply to the last message shown, the person's,
    /// with its skill and what's attached to it.
    private func request(in state: State) -> AssistantRequest {
        let conversation = state.conversation.id
        let asked = state.conversation.messages.last
        var request = AssistantRequest(
            messages: Array(state.conversation.unsummarized), draft: state.conversation.draft, problem: state.problem,
            workspace: state.workspace, now: now, timeZone: timeZone,
            writes: AssistantWrites(conversation: conversation),
            skill: asked?.skill.flatMap { id in state.skills.first { $0.id == id } },
            attachments: (asked?.attachments ?? []).compactMap { attachment in
                attachments.file(attachment, conversation).map { ModelAttachment(attachment.kind, file: $0) }
            })
        request.summary = state.conversation.summary?.summary.text
        request.model = state.conversation.context.model
        return request
    }

    /// Checks the draft against the records as they are now.
    private func check(_ state: inout State) {
        state.problem = state.conversation.draft.flatMap { plan in
            state.workspace.problem(with: plan, timeZone: timeZone, now: now)?.description
        }
    }
}

/// The skills Lifr may pick for a message sent without one, and the one it
/// keeps when it picks none.
private struct SkillPick: Sendable {
    var candidates: [Skill]
    var kept: Skill?
}

extension ModelThinking {
    /// This, unless the model thought nothing worth showing.
    fileprivate var shown: ModelThinking? {
        text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : self
    }
}

extension ChatFeature {
    /// A reply's text as it's said: its Markdown's words without the marks.
    static func spoken(_ text: String) -> String {
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        let plain = (try? AttributedString(markdown: text, options: options)).map { String($0.characters) } ?? text
        return plain.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// The language `text` is written in, as a BCP 47 tag: the device's
    /// when it can't be told.
    static func language(of text: String) -> String {
        if let language = NLLanguageRecognizer.dominantLanguage(for: text), language != .undetermined {
            return language.rawValue
        }
        return Locale.current.language.languageCode?.identifier ?? "en"
    }
}
