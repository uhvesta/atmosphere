import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrCore
import LifrSearchClient
import LifrStoreClient

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
        /// The latest records shown, so earlier ones arriving late are ignored.
        package var saves = SaveOrder()
        /// The open chat, and chats whose replies are still being written.
        package var chats: IdentifiedArrayOf<ChatFeature.State> = []
        /// The ID of the chat on screen.
        package var openID: UUID?
        /// Whether the archive is on screen.
        package var showsArchive = false
        /// What's searched for in the main list's names and messages, as
        /// typed.
        package var query = ""
        /// The query read: its words, and when the messages were written.
        package var searched = ConversationQuery(words: [])
        /// Words related to what's searched for, to find conversations by
        /// what they mean as well as what they say.
        package var related: [String] = []
        /// Whether related words are being found.
        package var relating = false
        /// Where related words come from: the system's word lists, found as
        /// the person types, or a model, asked when they submit the search.
        package var relatedSource = RelatedWordsSource.builtIn
        /// Whether related words were found for this search, even if none.
        package var hasRelated = false
        @Presents package var rename: RenameFeature.State?
        @Presents package var confirm: ConfirmationDialogState<Confirm>?
        @Presents package var alert: AlertState<Never>?
        /// Why the conversations couldn't be loaded, until they load.
        package var loadError: String?
        /// The chat open when Lifr last ran, with what was typed in it.
        @Shared(.openChat) package var lastOpen

        package init() {}

        /// The chat open now, with what's typed in it.
        package var open: OpenChat? {
            openID.map { OpenChat(id: $0, input: chats[id: $0]?.input ?? "") }
        }

        /// The main list: conversations not archived, newest first.
        package var sorted: [Conversation] {
            conversations.filter { !$0.isArchived }
                .sorted { ($0.updatedAt, $0.id.uuidString) > ($1.updatedAt, $1.id.uuidString) }
        }

        /// The main list's conversations that have the query, newest first,
        /// with where: those with its words before those with related ones.
        package var found: [Found] {
            let found = sorted.compactMap { conversation in
                conversation.match(searched, related: related).map {
                    Found(conversation: conversation, message: $0.message, isRelated: $0.isRelated)
                }
            }
            return found.filter { !$0.isRelated } + found.filter(\.isRelated)
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

        /// The archive's conversations that have the query, or all of it
        /// when nothing's searched for.
        package var archivedFound: [Conversation] {
            guard !query.isEmpty else { return archived }
            return archived.filter { $0.match(searched, related: related) != nil }
        }

        /// The chat on screen.
        package var chat: ChatFeature.State? { openID.flatMap { chats[id: $0] } }

        /// Shows `snapshot`'s records, here and in each chat, unless later
        /// ones are shown already.
        package mutating func show(_ snapshot: Snapshot) {
            guard saves.isLatest(snapshot) else { return }
            workspace = snapshot.workspace
            for id in chats.ids { chats[id: id]?.show(snapshot) }
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

        /// Lets go of chats that are neither on screen nor working, nor
        /// have something typed that isn't sent yet.
        mutating func prune() {
            let openID = openID
            chats.removeAll { $0.id != openID && !$0.isWorking && $0.input.isEmpty && $0.recordings.isEmpty }
        }
    }

    /// A conversation a search found, and the message that has what was
    /// searched for; nil when its name does.
    package struct Found: Identifiable, Equatable {
        package var conversation: Conversation
        package var message: ChatMessage?
        /// Whether it was found by words related to what was searched for.
        package var isRelated = false

        package var id: UUID { conversation.id }

        package init(conversation: Conversation, message: ChatMessage? = nil, isRelated: Bool = false) {
            self.conversation = conversation
            self.message = message
            self.isRelated = isRelated
        }
    }

    package enum Confirm: Equatable, Sendable {
        case delete(UUID)
        case deleteArchived
    }

    package enum Action {
        case task
        case retryTapped
        case loaded(Result<[Conversation], any Error>)
        case newTapped
        case opened(UUID)
        case searched(String)
        /// Finds words related to what's searched for, when they come from
        /// a model and so wait to be asked for.
        case relatedTapped
        case related([String])
        /// Opens a conversation a search found, on the branch with the
        /// message `messageID` when there is one.
        case foundTapped(UUID, messageID: UUID?)
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
        /// A save found a newer copy, written on another device, and kept
        /// that one.
        case newerKept(Conversation)
        case deleteFailed(any Error)
        case chats(IdentifiedActionOf<ChatFeature>)
        case confirm(PresentationAction<Confirm>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Snapshot)
        }
    }

    @Dependency(\.conversations) private var conversations
    @Dependency(\.attachments) private var attachments
    @Dependency(\.uuid) private var uuid
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone
    @Dependency(\.relatedWords) private var relatedWords
    @Dependency(\.continuousClock) private var clock
    @Dependency(\.saveQueue) private var saves

    private enum CancelID { case related }

    package init() {}

    package var body: some ReducerOf<Self> {
        CombineReducers { chats }
            .onChange(of: \.open) { _, open in
                Reduce { state, _ in
                    state.$lastOpen.withLock { $0 = open }
                    return .none
                }
            }
    }

    @ReducerBuilder<State, Action> private var chats: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task, .retryTapped:
                return .run { [conversations] send in
                    await send(.loaded(Result { try await conversations.load() }))
                }

            case .loaded(.success(let loaded)):
                state.loadError = nil
                state.conversations = IdentifiedArray(uniqueElements: loaded)
                for chat in state.chats where chat.conversation.isKept {
                    state.conversations[id: chat.id] = chat.conversation
                }
                // Back where the person was before the system ended Lifr.
                if state.openID == nil, let last = state.lastOpen, let conversation = state.conversations[id: last.id] {
                    state.open(conversation)
                    state.chats[id: last.id]?.input = last.input
                }
                return .none

            case .newTapped, .chats(.element(_, .delegate(.new))):
                state.open(Conversation(id: uuid(), createdAt: now))
                return .none

            case .opened(let id):
                guard let conversation = state.conversations[id: id] else { return .none }
                state.open(conversation)
                return .none

            case .searched(let query):
                guard query != state.query else { return .none }
                var calendar = Calendar(identifier: .gregorian)
                calendar.timeZone = timeZone
                state.query = query
                state.searched = ConversationQuery(query, now: now, calendar: calendar)
                state.related = []
                state.relating = false
                state.hasRelated = false
                state.relatedSource = relatedWords.source()
                guard state.relatedSource == .builtIn, !state.searched.words.isEmpty else {
                    return .cancel(id: CancelID.related)
                }
                // The word lists answer at once, so they're found as the
                // person types, once they pause.
                return relate(&state, after: .milliseconds(250))

            case .relatedTapped:
                guard !state.searched.words.isEmpty, !state.relating, !state.hasRelated else { return .none }
                return relate(&state, after: .zero)

            case .related(let words):
                state.relating = false
                state.hasRelated = true
                state.related = words
                return .none

            case .foundTapped(let id, let messageID):
                guard var conversation = state.conversations[id: id] else { return .none }
                // A chat writing a reply stays where it is.
                guard let messageID, state.chats[id: id]?.canGo ?? true,
                    !conversation.messages.contains(where: { $0.id == messageID })
                else {
                    state.open(conversation)
                    return .none
                }
                conversation.go(to: conversation.newest(through: messageID))
                state.conversations[id: id] = conversation
                state.chats[id: id]?.conversation = conversation
                state.open(conversation)
                return save(conversation)

            case .closed:
                state.openID = nil
                state.prune()
                return .none

            case .resumed:
                return .merge(
                    state.chats.filter { $0.stopped }.map { .send(.chats(.element(id: $0.id, action: .resume))) }
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

            case .unarchiveTapped(let id), .chats(.element(let id, .delegate(.unarchive))):
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

            case .loaded(.failure(let error)):
                state.loadError = String(describing: error)
                return .none

            case .newerKept(let newer):
                // A chat writing a reply keeps it: the reply is newer still,
                // so it's kept over that copy when it's done.
                guard state.conversations[id: newer.id] != nil, state.chats[id: newer.id]?.isWorking != true else {
                    return .none
                }
                state.conversations[id: newer.id] = newer
                state.chats[id: newer.id]?.conversation = newer
                state.alert = AlertState {
                    TextState("Changed on Another Device")
                } message: {
                    TextState(
                        "“\(newer.title)” has newer messages from another device, so that copy is shown. "
                            + "Make your change again if it's gone.")
                }
                return .none

            case .failed(let error):
                state.alert = AlertState {
                    TextState("Couldn't save the conversation")
                } message: {
                    TextState(error.localizedDescription)
                }
                return .none

            case .deleteFailed(let error):
                state.alert = AlertState {
                    TextState("Couldn't delete the conversation")
                } message: {
                    TextState(String(describing: error))
                }
                // The rows were taken out before the store answered.
                return .send(.task)

            case .chats(.element(_, .delegate(.fork(let fork)))):
                state.conversations[id: fork.id] = fork
                state.open(fork)
                return save(fork)

            case .chats(.element(_, .delegate(.saved(let snapshot)))):
                state.show(snapshot)
                return .send(.delegate(.saved(snapshot)))

            case .chats, .rename, .confirm, .alert, .delegate:
                return .none
            }
        }
        .forEach(\.chats, action: \.chats) { ChatFeature() }
        .ifLet(\.$rename, action: \.rename) { RenameFeature() }
        .ifLet(\.$confirm, action: \.confirm)
        .ifLet(\.$alert, action: \.alert)
        // Keep each change to a chat, on screen or not. A chat with no
        // messages, draft or added plan isn't kept. A chat that finished off
        // screen is let go.
        Reduce { state, action in
            let changed = state.chats.map(\.conversation).filter {
                $0.isKept && state.conversations[id: $0.id] != $0
            }
            for conversation in changed { state.conversations[id: conversation.id] = conversation }
            if case .chats = action { state.prune() }
            guard !changed.isEmpty else { return .none }
            return .merge(changed.map(save))
        }
    }

    /// Finds words related to what's searched for, after `delay`, in place
    /// of any being found for an earlier search.
    private func relate(_ state: inout State, after delay: Duration) -> Effect<Action> {
        let text = state.searched.text
        state.relating = true
        return .run { [relatedWords, clock] send in
            if delay > .zero { try await clock.sleep(for: delay) }
            await send(.related(relatedWords.related(text)))
        }
        .cancellable(id: CancelID.related, cancelInFlight: true)
    }

    /// Keeps `conversation`, or says a newer copy was kept instead.
    private func save(_ conversation: Conversation) -> Effect<Action> {
        let newer = LockIsolated<Conversation?>(nil)
        let change = saves.enqueue(saving: conversation.id) { [conversations] in
            let kept = try await conversations.save(conversation)
            newer.setValue(kept)
        }
        return .run { send in
            try await change.value
            if let newer = newer.value { await send(.newerKept(newer)) }
        } catch: { error, send in
            await send(.failed(error))
        }
    }

    private func delete(_ ids: [UUID], in state: inout State) -> Effect<Action> {
        state.remove(ids)
        return kept(
            failed: { .deleteFailed($0) },
            { [conversations, attachments] in
                try await conversations.delete(ids)
                for id in ids { await attachments.removeAll(id) }
            })
    }

    /// Changes the kept chats after the changes made before this one, and
    /// says if it couldn't.
    private func kept(
        saving chat: UUID? = nil,
        failed: @escaping @Sendable (any Error) -> Action = { .failed($0) },
        _ work: @escaping @Sendable () async throws -> Void
    ) -> Effect<Action> {
        let change = saves.enqueue(saving: chat, work)
        return .run { _ in
            try await change.value
        } catch: { error, send in
            await send(failed(error))
        }
    }

    private func deleteMessage(synced: Bool) -> String {
        synced
            ? String(localized: "It's deleted here and from iCloud, and can't be undone. Tasks it changed stay.")
            : String(localized: "This can't be undone. Tasks the assistant added or changed stay.")
    }
}

/// A chat open on screen, with what's typed in it and not sent.
package struct OpenChat: Codable, Equatable, Sendable {
    package var id: UUID
    package var input: String

    package init(id: UUID, input: String) {
        self.id = id
        self.input = input
    }
}

extension SharedKey where Self == FileStorageKey<OpenChat?>.Default {
    /// The chat open when Lifr last ran, on this device.
    package static var openChat: Self {
        Self[.fileStorage(.applicationSupportDirectory.appending(path: "Lifr/open-chat.json")), default: nil]
    }
}

extension Conversation {
    /// Whether there's anything to keep: a message, or a plan pasted in and
    /// still to review or already added.
    fileprivate var isKept: Bool { !messages.isEmpty || context.draft != nil || !context.added.isEmpty }
}
