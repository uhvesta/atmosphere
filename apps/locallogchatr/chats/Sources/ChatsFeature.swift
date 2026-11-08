import ChatFeatures
import ChatKit
import ComposableArchitecture
import Foundation
import LocalLogChatrModels

/// The chats on the device, newest first, and the one that's open. Each
/// keeps writing its reply when another is opened, and is saved whenever
/// it changes.
@Reducer
package struct ChatsFeature {
    @ObservableState
    package struct State: Equatable {
        package var chats: IdentifiedArrayOf<ChatThreadFeature.State> = []
        package var openID: UUID?
        /// Words to find chats by, in their names and messages.
        package var query = ""
        /// The chat being renamed, and its name as it's typed.
        package var renaming: UUID?
        package var name = ""
        /// The chat to delete, once the person says so.
        package var deleting: UUID?
        /// Why a chat couldn't be saved, read or deleted.
        package var problem: String?
        @Presents package var settings: SettingsFeature.State?

        package init() {}

        /// The chats, the one written in last first, with what's searched for.
        package var shown: [ChatThreadFeature.State] {
            let words = query.split(separator: " ").map(String.init)
            return chats.filter { chat in
                words.allSatisfy { word in
                    chat.thread.title.localizedStandardContains(word)
                        || chat.thread.messages.contains { $0.text.localizedStandardContains(word) }
                }
            }
            .sorted { $0.thread.updatedAt > $1.thread.updatedAt }
        }

        package var open: ChatThreadFeature.State? { openID.flatMap { chats[id: $0] } }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case loaded([SavedChat], unreadable: [String])
        case newTapped
        case openTapped(UUID)
        case closed
        case renameTapped(UUID)
        case renameSaved
        case renameCancelled
        case deleteTapped(UUID)
        case deleteConfirmed
        case deleteCancelled
        case failed(String)
        case problemDismissed
        case settingsTapped
        case settings(PresentationAction<SettingsFeature.Action>)
        case chats(IdentifiedActionOf<ChatThreadFeature>)
    }

    /// What every chat knows before the person's first message.
    package static let instructions = """
        You're a helpful assistant running on this device, so nothing said here leaves it. Answer in the language \
        of the person's last message. Be clear and to the point, and use Markdown when it helps: lists, tables \
        and code blocks.
        """

    @Dependency(\.chatShelf) private var shelf
    @Dependency(\.chatPhotos) private var chatPhotos
    @Dependency(\.date.now) private var now
    @Dependency(\.uuid) private var uuid

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .task:
                guard state.chats.isEmpty else { return .none }
                return .run { [shelf] send in
                    let (chats, unreadable) = shelf.chats()
                    await send(.loaded(chats, unreadable: unreadable.map(\.description)))
                }

            case .loaded(let saved, let unreadable):
                for chat in saved where state.chats[id: chat.id] == nil {
                    state.chats.append(Self.chat(chat))
                }
                state.problem = unreadable.first
                return .none

            case .newTapped:
                // An empty chat is opened again, not made twice.
                if let empty = state.chats.first(where: { $0.thread.messages.isEmpty }) {
                    state.openID = empty.id
                    return .none
                }
                let chat = Self.chat(SavedChat(conversation: ChatThread(id: uuid(), createdAt: now)))
                state.chats.append(chat)
                state.openID = chat.id
                return .none

            case .openTapped(let id):
                state.openID = state.chats[id: id]?.id
                return .none

            case .closed:
                state.openID = nil
                return .none

            case .renameTapped(let id):
                guard let chat = state.chats[id: id] else { return .none }
                state.renaming = id
                state.name = chat.thread.name ?? ""
                return .none

            case .renameSaved:
                guard let id = state.renaming else { return .none }
                state.renaming = nil
                let name = state.name
                state.chats[id: id]?.thread.rename(name)
                return save(id, in: state)

            case .renameCancelled:
                state.renaming = nil
                return .none

            case .deleteTapped(let id):
                state.deleting = id
                return .none

            case .deleteConfirmed:
                guard let id = state.deleting, let chat = state.chats[id: id] else { return .none }
                state.deleting = nil
                if state.openID == id { state.openID = nil }
                state.chats.remove(id: id)
                return .run { [shelf, chatPhotos] send in
                    do throws(ChatShelfError) {
                        try shelf.delete(SavedChat(conversation: chat.thread, model: chat.model))
                        // Its photos go once the chat is gone, so none is left without them.
                        await chatPhotos.removeAll(id)
                    } catch {
                        await send(.failed(error.description))
                    }
                }

            case .deleteCancelled:
                state.deleting = nil
                return .none

            case .failed(let problem):
                state.problem = problem
                return .none

            case .problemDismissed:
                state.problem = nil
                return .none

            case .settingsTapped:
                state.settings = SettingsFeature.State()
                return .none

            case .chats(.element(let id, .delegate(.changed))):
                return save(id, in: state)

            case .binding, .settings, .chats:
                return .none
            }
        }
        .forEach(\.chats, action: \.chats) { ChatThreadFeature() }
        .ifLet(\.$settings, action: \.settings) { SettingsFeature() }
    }

    /// Keeps the chat as it is now.
    private func save(_ id: UUID, in state: State) -> Effect<Action> {
        guard let chat = state.chats[id: id] else { return .none }
        let saved = SavedChat(conversation: chat.thread, model: chat.model)
        return .run { [shelf] send in
            do throws(ChatShelfError) {
                try shelf.save(saved)
            } catch {
                await send(.failed(error.description))
            }
        }
    }

    /// The chat to show for one that's kept.
    static func chat(_ saved: SavedChat) -> ChatThreadFeature.State {
        ChatThreadFeature.State(thread: saved.conversation, model: saved.model, use: .chat, instructions: instructions)
    }
}
