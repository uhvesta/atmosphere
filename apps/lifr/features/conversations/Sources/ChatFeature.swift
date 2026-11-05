import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrCore
import LifrModelsClient
import LifrSkills
import LifrStoreClient

/// A chat with the assistant: it answers questions, changes tasks when
/// asked (each reply that did has Undo), and drafts plans on a card the
/// person can edit and then add. A reply keeps going when the person
/// leaves the chat or Lifr; one the system turns away, as Apple's model
/// does in the background, is sent again on `resume`.
@Reducer
package struct ChatFeature {
    @ObservableState
    package struct State: Equatable, Identifiable {
        package var conversation: Conversation
        package var workspace: Workspace
        package var input = ""
        package var availability = AssistantAvailability.available
        /// The reply as it's written, until it's complete.
        package var replying: String?
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
        /// Why the draft can't be added as it is now, checked on every
        /// change, and sent with the next message.
        package var problem: String?
        /// The day the draft's dates are read from, such as "saturday".
        package var today: CalendarDate?
        @Presents package var alert: AlertState<Action.Alert>?
        /// The request of a reply that failed in a way sending it again can
        /// fix, for the alert's Try Again.
        package var failed: AssistantRequest?
        /// Every skill, on or off. The built-ins until the person's load.
        package var skills = Skill.builtIn
        /// The ID of the skill picked for the next message.
        package var skill: String?
        /// Whether the skill finder is open, and what's typed in it.
        package var findsSkill = false
        package var skillQuery = ""
        @Presents package var manageSkills: SkillsFeature.State?
        /// Whether the draft's editor is open.
        package var editsDraft = false
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

        package init(conversation: Conversation, workspace: Workspace) {
            self.conversation = conversation
            self.workspace = workspace
        }

        package var id: UUID { conversation.id }

        /// Whether a reply is being written, waits to be sent again, is
        /// being finished, or failed with an alert not yet seen: the chat is
        /// kept until it isn't.
        package var isWorking: Bool { replying != nil || waiting != nil || reloading || naming || alert != nil }

        package var canAdd: Bool { conversation.draft != nil && problem == nil }

        /// The skill picked for the next message, while it exists and is on.
        package var picked: Skill? { skill.flatMap { id in skills.first { $0.id == id && $0.isOn } } }

        /// The skills that are on, matching the finder.
        package var skillChoices: [Skill] { Skill.search(skillQuery, in: skills.filter(\.isOn)) }

        /// The title of the skill a message was sent with, if any.
        package func skillTitle(of message: ChatMessage) -> String? {
            message.skill.map { id in skills.first { $0.id == id }?.title ?? id }
        }

        package var canSend: Bool {
            availability == .available && replying == nil && waiting == nil && !attaching && !recording
                && (picked != nil || !pending.isEmpty
                    || !input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case skillsLoaded([Skill])
        case findSkillTapped
        case pickSkillTapped(String)
        case clearSkillTapped
        case manageSkillsTapped
        case manageSkills(PresentationAction<SkillsFeature.Action>)
        case sendTapped
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
        case replied(AssistantReply)
        case replyFailed(any Error)
        case reloaded(messageID: UUID, Result<Workspace, any Error>)
        case named(Result<String, any Error>)
        /// The title was tapped, to rename the conversation.
        case renameTapped
        case undoTapped(messageID: UUID)
        case discardDraftTapped
        case addTapped
        case saved(Result<Saved, any Error>)
        case alert(PresentationAction<Alert>)
        case delegate(Delegate)

        package enum Alert: Equatable {
            case tryAgain
        }

        @CasePathable
        package enum Delegate {
            case saved(Workspace)
            case rename
        }
    }

    private enum CancelID { case reply, recording }

    /// How long the assistant may write nothing before Lifr stops waiting:
    /// it gives up between one and two of these after the last progress.
    package static let stallLimit = Duration.seconds(60)

    @Dependency(\.assistant) private var assistant
    @Dependency(\.store) private var store
    @Dependency(\.skills) private var skillsClient
    @Dependency(\.guidance) private var guidance
    @Dependency(\.attachments) private var attachments
    @Dependency(\.recorder) private var recorder
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
                state.availability = assistant.availability()
                state.today = CalendarDate(now, in: timeZone)
                if state.availability == .available { assistant.prewarm() }
                state.inputs = assistant.inputs()
                let conversation = state.conversation.id
                state.elsewhere = Set(
                    state.conversation.messages.flatMap(\.attachments)
                        .filter { attachments.file($0, conversation) == nil }.map(\.id))
                return .run { [skillsClient] send in
                    // Without the person's skills, the built-ins still work.
                    if let skills = try? await skillsClient.load() { await send(.skillsLoaded(skills)) }
                }

            case .skillsLoaded(let skills):
                state.skills = skills
                return .none

            case .findSkillTapped:
                state.skillQuery = ""
                state.findsSkill = true
                return .none

            case .pickSkillTapped(let id):
                state.findsSkill = false
                guard state.skills.contains(where: { $0.id == id && $0.isOn }) else { return .none }
                state.skill = id
                return .none

            case .clearSkillTapped:
                state.skill = nil
                return .none

            case .manageSkillsTapped:
                state.findsSkill = false
                state.manageSkills = SkillsFeature.State()
                return .none

            case .manageSkills(.dismiss):
                return .run { [skillsClient] send in
                    if let skills = try? await skillsClient.load() { await send(.skillsLoaded(skills)) }
                }

            case .sendTapped:
                guard state.canSend else { return .none }
                let text = state.input.trimmingCharacters(in: .whitespacesAndNewlines)
                state.input = ""
                let skill = state.picked
                state.skill = nil
                let sent = state.pending
                state.pending = []
                let conversation = state.conversation.id
                state.conversation.messages.append(
                    ChatMessage(id: uuid(), role: .person, text: text, at: now, skill: skill?.id, attachments: sent))
                state.replying = ""
                let request = AssistantRequest(
                    messages: state.conversation.messages, draft: state.conversation.draft, problem: state.problem,
                    workspace: state.workspace, now: now, timeZone: timeZone,
                    writes: AssistantWrites(conversation: state.conversation.id) { [store] operation in
                        try await store.save(operation).workspace
                    },
                    skill: skill,
                    attachments: sent.compactMap { attachment in
                        attachments.file(attachment, conversation).map { ModelAttachment(attachment.kind, file: $0) }
                    })
                return reply(request, in: &state)

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

            case .resume:
                // A reply still being written carries on by itself.
                guard state.stopped, let request = state.waiting else { return .none }
                return reply(request, in: &state)

            case .replied(let reply):
                guard reply.isComplete else {
                    state.replying = reply.text
                    return .none
                }
                state.replying = nil
                state.waiting = nil
                state.stopped = false
                state.reloading = true
                let id = uuid()
                state.conversation.messages.append(ChatMessage(id: id, role: .assistant, text: reply.text, at: now))
                if let plan = reply.plan {
                    state.conversation.draft = plan
                    check(&state)
                }
                let reload: Effect<Action> = .run { [store] send in
                    await send(.reloaded(messageID: id, Result { try await store.load() }))
                }
                guard state.namesConversations, !state.conversation.isNamedByHand, state.conversation.name == nil
                else { return reload }
                state.naming = true
                let messages = state.conversation.messages
                return .merge(
                    reload,
                    .run { [assistant] send in
                        await send(.named(Result { try await assistant.suggestTitle(messages) }))
                    })

            case .named(let result):
                state.naming = false
                // A failed name leaves the first message as the title.
                guard case .success(let name) = result, !state.conversation.isNamedByHand else { return .none }
                state.conversation.name = name
                return .none

            case .renameTapped:
                return .send(.delegate(.rename))

            case .replyFailed(let error as AssistantError) where error == .busy:
                // Keep what's written so far, and send again on `resume`.
                state.stopped = true
                return .none

            case .replyFailed(let error):
                state.replying = nil
                // Sending again can't help a request that's too long or
                // declined, and could make saved changes twice.
                let retries =
                    (error as? AssistantError).map {
                        $0 == .unreadable(saved: false) || $0 == .timedOut(saved: false) || $0.isFailure
                    } ?? true
                state.failed = retries ? state.waiting : nil
                state.waiting = nil
                state.stopped = false
                state.alert = AlertState {
                    TextState("The assistant couldn't answer")
                } actions: {
                    if retries {
                        ButtonState(action: .tryAgain) { TextState("Try Again") }
                        ButtonState(role: .cancel) { TextState("Cancel") }
                    } else {
                        ButtonState(role: .cancel) { TextState("OK") }
                    }
                } message: {
                    TextState(String(describing: error))
                }
                return .none

            case .alert(.presented(.tryAgain)):
                guard let request = state.failed else { return .none }
                state.failed = nil
                return reply(request, in: &state)

            case .alert(.dismiss):
                state.failed = nil
                return .none

            case .reloaded(let messageID, .success(let workspace)):
                state.reloading = false
                let known = Set(state.conversation.changes.values.joined()).union(state.conversation.added)
                let new = workspace.entries(inConversation: state.conversation.id).map(\.id)
                    .filter { !known.contains($0) }
                guard !new.isEmpty else { return .none }
                state.conversation.changes[messageID] = new
                state.workspace = workspace
                check(&state)
                return .send(.delegate(.saved(workspace)))

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

            case .discardDraftTapped:
                state.conversation.draft = nil
                state.problem = nil
                state.editsDraft = false
                return .none

            case .binding(\.conversation.draft):
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
                state.workspace = saved.workspace
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
                    }
                }
                check(&state)
                return .send(.delegate(.saved(saved.workspace)))

            case .saved(.failure(let error)):
                state.problem = String(describing: error)
                state.alert = AlertState {
                    TextState("Couldn't save")
                } message: {
                    TextState(
                        "\(error) Nothing changed. Send a message and the assistant will fix the draft.")
                }
                return .none

            case .binding, .alert, .manageSkills, .delegate:
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
        .ifLet(\.$manageSkills, action: \.manageSkills) { SkillsFeature() }
    }

    /// Writes the reply to `request`, with background time so it keeps
    /// going when the person leaves Lifr. Until it's complete, `waiting`
    /// keeps the request for `resume`.
    private func reply(_ request: AssistantRequest, in state: inout State) -> Effect<Action> {
        state.waiting = request
        state.stopped = false
        if state.replying == nil { state.replying = "" }
        return .run { [assistant, backgroundTime, clock, guidance] send in
            var request = request
            // The default when the person's can't be read; the
            // Instructions screen says why.
            request.guidance = (try? await guidance.load().text) ?? Guidance.defaultText
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
    /// assistant writes or saves nothing for `stallLimit`. Time in the
    /// background doesn't count: a model there may be waiting for Lifr to
    /// be back on screen, so each return starts the wait again.
    private static func watch(
        _ request: AssistantRequest, assistant: AssistantClient, clock: any Clock<Duration>,
        returns: @escaping @Sendable () -> Int, _ each: @escaping @Sendable (AssistantReply) async -> Void
    ) async throws {
        let progress = LockIsolated(0)
        let saved = LockIsolated(false)
        var request = request
        if let writes = request.writes {
            request.writes = AssistantWrites(conversation: writes.conversation) { operation in
                progress.withValue { $0 += 1 }
                defer { progress.withValue { $0 += 1 } }
                let workspace = try await writes.save(operation)
                saved.setValue(true)
                return workspace
            }
        }
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
                throw AssistantError.timedOut(saved: saved.value)
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

    /// Checks the draft against the records as they are now.
    private func check(_ state: inout State) {
        state.problem = state.conversation.draft.flatMap { plan in
            state.workspace.problem(with: plan, timeZone: timeZone, now: now)?.description
        }
    }
}
