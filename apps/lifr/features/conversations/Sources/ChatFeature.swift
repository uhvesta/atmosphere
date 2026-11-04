import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrCore
import LifrStoreClient

/// A chat with the assistant: it answers questions, changes tasks when
/// asked (each reply that did has Undo), and drafts plans on a card the
/// person adds.
@Reducer
package struct ChatFeature {
    @ObservableState
    package struct State: Equatable {
        package var conversation: Conversation
        package var workspace: Workspace
        package var input = ""
        package var availability = AssistantAvailability.available
        /// The reply as it's written, until it's complete.
        package var replying: String?
        /// Why the draft can't be added, sent with the next message.
        package var problem: String?
        @Presents package var alert: AlertState<Never>?

        package init(conversation: Conversation, workspace: Workspace) {
            self.conversation = conversation
            self.workspace = workspace
        }

        package var canSend: Bool {
            availability == .available && replying == nil
                && !input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case sendTapped
        case replied(AssistantReply)
        case replyFailed(any Error)
        case reloaded(messageID: UUID, Result<Workspace, any Error>)
        case undoTapped(messageID: UUID)
        case discardDraftTapped
        case addTapped
        case saved(Result<Saved, any Error>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Workspace)
        }
    }

    private enum CancelID { case reply }

    @Dependency(\.assistant) private var assistant
    @Dependency(\.store) private var store
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone
    @Dependency(\.uuid) private var uuid

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .task:
                state.availability = assistant.availability()
                return .none

            case .sendTapped:
                guard state.canSend else { return .none }
                let text = state.input.trimmingCharacters(in: .whitespacesAndNewlines)
                state.input = ""
                state.conversation.messages.append(ChatMessage(id: uuid(), role: .person, text: text, at: now))
                state.replying = ""
                let request = AssistantRequest(
                    messages: state.conversation.messages, draft: state.conversation.draft, problem: state.problem,
                    workspace: state.workspace, now: now, timeZone: timeZone,
                    writes: AssistantWrites(conversation: state.conversation.id) { [store] operation in
                        try await store.save(operation).workspace
                    })
                return .run { [assistant] send in
                    for try await reply in assistant.reply(request) { await send(.replied(reply)) }
                } catch: { error, send in
                    await send(.replyFailed(error))
                }
                .cancellable(id: CancelID.reply, cancelInFlight: true)

            case .replied(let reply):
                guard reply.isComplete else {
                    state.replying = reply.text
                    return .none
                }
                state.replying = nil
                let id = uuid()
                state.conversation.messages.append(ChatMessage(id: id, role: .assistant, text: reply.text, at: now))
                if let plan = reply.plan {
                    state.conversation.draft = plan
                    state.problem = nil
                }
                return .run { [store] send in
                    await send(.reloaded(messageID: id, Result { try await store.load() }))
                }

            case .replyFailed(let error):
                state.replying = nil
                state.alert = AlertState {
                    TextState("The assistant couldn't answer")
                } message: {
                    TextState(String(describing: error))
                }
                return .none

            case .reloaded(let messageID, .success(let workspace)):
                let known = Set(state.conversation.changes.values.joined()).union(state.conversation.added)
                let new = workspace.entries(inConversation: state.conversation.id).map(\.id)
                    .filter { !known.contains($0) }
                guard !new.isEmpty else { return .none }
                state.conversation.changes[messageID] = new
                state.workspace = workspace
                return .send(.delegate(.saved(workspace)))

            case .reloaded(_, .failure):
                // The assistant's changes are saved; the screens catch up on
                // the next update.
                return .none

            case .undoTapped(let messageID):
                guard let entries = state.conversation.changes[messageID] else { return .none }
                return .run { [store] send in
                    await send(.saved(Result { try await store.revert(entries: entries) }))
                }

            case .discardDraftTapped:
                state.conversation.draft = nil
                state.problem = nil
                return .none

            case .addTapped:
                guard let plan = state.conversation.draft else { return .none }
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
                    state.problem = nil
                    if let entry = saved.change.entry {
                        state.conversation.added.append(entry)
                    }
                }
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

            case .binding, .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }
}
