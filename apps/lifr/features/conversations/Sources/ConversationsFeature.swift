import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrCore

/// The conversations with the assistant, newest first, and the open one.
/// Each is kept as it changes, on this device until it's synced. Archived
/// ones move to the archive, the only place they can be deleted; deleting
/// one never deletes tasks. A chat
/// stays here while it's open or its reply is still being written, so
/// leaving it or opening another doesn't stop the reply.
@Reducer
package struct ConversationsFeature {
    @ObservableState
    package struct State: Equatable {
        package var conversations: IdentifiedArrayOf<Conversation> = []
        package var workspace = Workspace()
        /// The open chat, and chats whose replies are still being written.
        package var chats: IdentifiedArrayOf<ChatFeature.State> = []
        /// The ID of the chat on screen.
        package var openID: UUID?
        /// Whether the archive is on screen.
        package var showsArchive = false
        @Presents package var rename: RenameFeature.State?
        @Presents package var confirm: ConfirmationDialogState<Confirm>?
        @Presents package var alert: AlertState<Never>?

        package init() {}

        /// The main list: conversations not archived, newest first.
        package var sorted: [Conversation] {
            conversations.filter { !$0.isArchived }
                .sorted { ($0.updatedAt, $0.id.uuidString) > ($1.updatedAt, $1.id.uuidString) }
        }

        /// The archive, most recently archived first.
        package var archived: [Conversation] {
            conversations.filter(\.isArchived)
                .sorted {
                    ($0.archivedAt ?? .distantPast, $0.id.uuidString) > (
                        $1.archivedAt ?? .distantPast, $1.id.uuidString
                    )
                }
        }

        /// The chat on screen.
        package var chat: ChatFeature.State? { openID.flatMap { chats[id: $0] } }

        package mutating func show(_ workspace: Workspace) {
            self.workspace = workspace
            for id in chats.ids { chats[id: id]?.workspace = workspace }
        }

        /// Opens `conversation`, keeping its chat if it's still working.
        mutating func open(_ conversation: Conversation) {
            if chats[id: conversation.id] == nil {
                chats.append(ChatFeature.State(conversation: conversation, workspace: workspace))
            }
            openID = conversation.id
            prune()
        }

        /// Changes the conversation and its chat's copy alike; nil when it
        /// isn't here.
        mutating func update(_ id: UUID, _ change: (inout Conversation) -> Void) -> Conversation? {
            guard var conversation = conversations[id: id] else { return nil }
            change(&conversation)
            conversations[id: id] = conversation
            chats[id: id]?.conversation = conversation
            return conversation
        }

        /// Removes conversations and their chats.
        mutating func remove(_ ids: [UUID]) {
            for id in ids {
                conversations.remove(id: id)
                chats.remove(id: id)
                if openID == id { openID = nil }
            }
        }

        /// Lets go of chats that are neither on screen nor working.
        mutating func prune() {
            let openID = openID
            chats.removeAll { $0.id != openID && !$0.isWorking }
        }
    }

    package enum Confirm: Equatable, Sendable {
        case delete(UUID)
        case deleteArchived
    }

    package enum Action {
        case task
        case loaded(Result<[Conversation], any Error>)
        case newTapped
        case opened(UUID)
        /// The chat on screen was left.
        case closed
        /// Lifr is back on screen: replies the system stopped go again.
        case resumed
        /// Syncs the conversation, or stops syncing it.
        case syncTapped(UUID)
        case archiveTapped(UUID)
        case unarchiveTapped(UUID)
        case archiveShown(Bool)
        case renameTapped(UUID)
        case rename(PresentationAction<RenameFeature.Action>)
        /// Asks before deleting an archived conversation for good.
        case deleteTapped(UUID)
        /// Asks before deleting every archived conversation.
        case deleteArchivedTapped
        case failed(any Error)
        case chats(IdentifiedActionOf<ChatFeature>)
        case confirm(PresentationAction<Confirm>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Workspace)
        }
    }

    @Dependency(\.conversations) private var conversations
    @Dependency(\.attachments) private var attachments
    @Dependency(\.uuid) private var uuid
    @Dependency(\.date.now) private var now

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                return .run { [conversations] send in
                    await send(.loaded(Result { try await conversations.load() }))
                }

            case .loaded(.success(let loaded)):
                state.conversations = IdentifiedArray(uniqueElements: loaded)
                for chat in state.chats where !chat.conversation.messages.isEmpty {
                    state.conversations[id: chat.id] = chat.conversation
                }
                return .none

            case .newTapped:
                state.open(Conversation(id: uuid(), createdAt: now))
                return .none

            case .opened(let id):
                guard let conversation = state.conversations[id: id] else { return .none }
                state.open(conversation)
                return .none

            case .closed:
                state.openID = nil
                state.prune()
                return .none

            case .resumed:
                return .merge(
                    state.chats.filter { $0.waiting != nil }.map { .send(.chats(.element(id: $0.id, action: .resume))) }
                )

            case .syncTapped(let id):
                guard let conversation = state.update(id, { $0.isSynced.toggle() }) else { return .none }
                return save(conversation)

            case .archiveTapped(let id):
                guard let conversation = state.update(id, { $0.archivedAt = now }) else { return .none }
                if state.openID == id {
                    state.openID = nil
                    state.prune()
                }
                return save(conversation)

            case .unarchiveTapped(let id):
                guard let conversation = state.update(id, { $0.archivedAt = nil }) else { return .none }
                return save(conversation)

            case .renameTapped(let id), .chats(.element(let id, .delegate(.rename))):
                guard let conversation = state.conversations[id: id] ?? state.chats[id: id]?.conversation else {
                    return .none
                }
                state.rename = RenameFeature.State(conversation)
                return .none

            case .rename(.presented(.delegate(.renamed(let id, let name)))):
                state.rename = nil
                if state.conversations[id: id] == nil {
                    // A new chat isn't kept until it has a message; name its
                    // copy, which is kept with that message.
                    state.chats[id: id]?.conversation.rename(name)
                    return .none
                }
                guard let conversation = state.update(id, { $0.rename(name) }) else { return .none }
                return save(conversation)

            case .archiveShown(let shown):
                state.showsArchive = shown
                return .none

            case .deleteTapped(let id):
                guard let conversation = state.conversations[id: id] else { return .none }
                state.confirm = ConfirmationDialogState {
                    TextState("Delete “\(conversation.title)”?")
                } actions: {
                    ButtonState(role: .destructive, action: .delete(id)) { TextState("Delete") }
                } message: {
                    TextState(deleteMessage(synced: conversation.isSynced))
                }
                return .none

            case .deleteArchivedTapped:
                let archived = state.archived
                guard !archived.isEmpty else { return .none }
                state.confirm = ConfirmationDialogState {
                    TextState("Delete all archived conversations?")
                } actions: {
                    ButtonState(role: .destructive, action: .deleteArchived) { TextState("Delete All") }
                } message: {
                    TextState(deleteMessage(synced: archived.contains(where: \.isSynced)))
                }
                return .none

            case .confirm(.presented(.delete(let id))):
                return delete([id], in: &state)

            case .confirm(.presented(.deleteArchived)):
                return delete(state.archived.map(\.id), in: &state)

            case .loaded(.failure(let error)), .failed(let error):
                state.alert = AlertState {
                    TextState("Conversations")
                } message: {
                    TextState(String(describing: error))
                }
                return .none

            case .chats(.element(_, .delegate(.saved(let workspace)))):
                state.show(workspace)
                return .send(.delegate(.saved(workspace)))

            case .chats, .rename, .confirm, .alert, .delegate:
                return .none
            }
        }
        .forEach(\.chats, action: \.chats) { ChatFeature() }
        .ifLet(\.$rename, action: \.rename) { RenameFeature() }
        .ifLet(\.$confirm, action: \.confirm)
        .ifLet(\.$alert, action: \.alert)
        // Keep each change to a chat, on screen or not. A chat with no
        // messages isn't kept. A chat that finished off screen is let go.
        Reduce { state, action in
            let changed = state.chats.map(\.conversation).filter {
                !$0.messages.isEmpty && state.conversations[id: $0.id] != $0
            }
            for conversation in changed { state.conversations[id: conversation.id] = conversation }
            if case .chats = action { state.prune() }
            guard !changed.isEmpty else { return .none }
            return .run { [conversations] _ in
                for conversation in changed { try await conversations.save(conversation) }
            } catch: { error, send in
                await send(.failed(error))
            }
        }
    }

    private func save(_ conversation: Conversation) -> Effect<Action> {
        .run { [conversations] _ in
            try await conversations.save(conversation)
        } catch: { error, send in
            await send(.failed(error))
        }
    }

    private func delete(_ ids: [UUID], in state: inout State) -> Effect<Action> {
        state.remove(ids)
        return .run { [conversations, attachments] _ in
            try await conversations.delete(ids)
            for id in ids { await attachments.removeAll(id) }
        } catch: { error, send in
            await send(.failed(error))
        }
    }

    private func deleteMessage(synced: Bool) -> String {
        synced
            ? String(localized: "It's deleted here and from iCloud, and can't be undone. Tasks it changed stay.")
            : String(localized: "This can't be undone. Tasks the assistant added or changed stay.")
    }
}
