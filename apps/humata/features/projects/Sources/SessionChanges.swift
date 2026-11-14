import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// Renames a session.
/// Moves a project's session to one of its tasks or subtasks, or to the
/// project itself.
@Reducer
package struct MoveSessionFeature {
    @ObservableState
    package struct State: Equatable {
        package var session: Session
        /// The project's items, tasks before their subtasks; nil until read.
        package var items: [Item]?
        /// Why the items couldn't be read, and what to do about it.
        package var itemsFailure: String?
        /// Where it goes: an item's id, or nil for the project itself.
        package var itemID: String?
        package var saving = false
        @Presents package var alert: AlertState<Never>?

        package init(session: Session) {
            self.session = session
            self.itemID = session.itemID
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case itemsLoaded([Item])
        case itemsFailed(String)
        case moveTapped
        case failed(String)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case moved(Session)
        }
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding, .alert, .delegate:
                return .none

            case .task:
                guard let project = state.session.projectID else { return .none }
                state.itemsFailure = nil
                return .run { send in
                    await send(.itemsLoaded(try await daemon.items(project)))
                } catch: { error, send in
                    await send(.itemsFailed(error.localizedDescription))
                }

            case .itemsFailed(let message):
                state.itemsFailure = message
                return .none

            case .itemsLoaded(let items):
                state.itemsFailure = nil
                let tasks = items.filter { $0.fields.parentID == nil }
                state.items = tasks.flatMap { task in
                    [task] + items.filter { $0.fields.parentID == task.id }
                }
                return .none

            case .moveTapped:
                guard !state.saving else { return .none }
                state.saving = true
                let id = state.session.id
                let placement = state.itemID.map(Placement.item) ?? .project
                return .run { send in
                    await send(.delegate(.moved(try await daemon.update(id, SessionUpdate(placement: placement)))))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .failed(let message):
                state.saving = false
                state.alert = AlertState {
                    TextState("Couldn't move the session")
                } message: {
                    TextState(message)
                }
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }
}

@Reducer
package struct RenameSessionFeature {
    @ObservableState
    package struct State: Equatable {
        package var session: Session
        package var name: String
        package var saving = false
        @Presents package var alert: AlertState<Never>?

        package init(session: Session) {
            self.session = session
            self.name = session.name
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case saveTapped
        case failed(String)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case renamed(Session)
        }
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding, .alert, .delegate:
                return .none

            case .saveTapped:
                guard !state.saving else { return .none }
                state.saving = true
                let (id, name) = (state.session.id, state.name.trimmingCharacters(in: .whitespacesAndNewlines))
                return .run { send in
                    await send(.delegate(.renamed(try await daemon.update(id, SessionUpdate(name: name)))))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .failed(let message):
                state.saving = false
                state.alert = AlertState {
                    TextState("Couldn't rename the session")
                } message: {
                    TextState(message)
                }
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }
}

/// Two sessions' changes side by side, to pick the one to keep.
@Reducer
package struct CompareFeature {
    @ObservableState
    package struct State: Equatable {
        package var left: Session
        package var right: Session
        /// Each session's changes by repository; nil until read.
        package var leftChanges: Changes?
        package var rightChanges: Changes?
        /// Why the changes couldn't be read, and what to do about it.
        package var failure: String?
        package var marking = false
        /// Why the session picked couldn't be marked as the one to keep;
        /// the changes stay shown, and it can be picked again.
        package var keepFailure: String?

        package init(left: Session, right: Session) {
            self.left = left
            self.right = right
        }
    }

    /// A session's changes as the lines to draw, by repository. A diff of a
    /// lockfile or a generated file runs to tens of thousands of lines, so
    /// only the first `limit` show until the rest are asked for.
    package struct Changes: Equatable, Sendable {
        /// One repository's patch.
        package struct Patch: Equatable, Sendable, Identifiable {
            /// The repository's name.
            package var name: String
            /// The patch, a line each.
            package var lines: [String]
            /// How many of `lines` show, from the first.
            package var shown: Int

            package var id: String { name }
        }

        /// The most lines shown before the rest are asked for.
        package static let limit = 5_000

        package var patches: [Patch]
        /// How many lines the patches have in all.
        package var lineCount: Int

        /// Splits each repository's patch, and shows the first `limit`
        /// lines across them.
        package init(_ changes: [RepositoryChanges]) {
            var left = Self.limit
            let patches = changes.map { repository -> Patch in
                var lines = repository.patch.split(separator: "\n", omittingEmptySubsequences: false)
                // A patch ends with a newline, which isn't a line of its own.
                if lines.last?.isEmpty == true { lines.removeLast() }
                let shown = min(lines.count, left)
                left -= shown
                return Patch(name: repository.name, lines: lines.map { String($0) }, shown: shown)
            }
            self.patches = patches
            lineCount = patches.reduce(0) { $0 + $1.lines.count }
        }

        package var isEmpty: Bool { lineCount == 0 }
        package var shownCount: Int { patches.reduce(0) { $0 + $1.shown } }
        /// How many lines wait to be asked for.
        package var hidden: Int { lineCount - shownCount }

        mutating func showAll() {
            for index in patches.indices { patches[index].shown = patches[index].lines.count }
        }
    }

    package enum Action {
        /// The sheet appeared; reads both sessions' changes.
        case task
        case loaded(left: Changes, right: Changes)
        case loadFailed(String)
        /// Shows every line of the session's changes, past the first ones.
        case showRestTapped(Session.ID)
        /// Marks the session as the keeper, and the other as not.
        case keepTapped(Session.ID)
        case marked([Session])
        case markFailed(String)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case updated(Session)
        }
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .delegate:
                return .none

            case .task:
                state.failure = nil
                let (left, right) = (state.left.id, state.right.id)
                return .run { send in
                    async let leftChanges = daemon.changes(left, ChangesQuery())
                    async let rightChanges = daemon.changes(right, ChangesQuery())
                    // Split here, off the main thread, so the view only draws.
                    let leftLines = Changes(try await leftChanges)
                    let rightLines = Changes(try await rightChanges)
                    await send(.loaded(left: leftLines, right: rightLines))
                } catch: { error, send in
                    await send(.loadFailed(error.localizedDescription))
                }

            case .loaded(let left, let right):
                state.leftChanges = left
                state.rightChanges = right
                return .none

            case .showRestTapped(let id):
                if id == state.left.id { state.leftChanges?.showAll() }
                if id == state.right.id { state.rightChanges?.showAll() }
                return .none

            case .loadFailed(let message):
                state.failure = message
                return .none

            case .markFailed(let message):
                state.marking = false
                state.keepFailure = message
                return .none

            case .keepTapped(let id):
                guard !state.marking else { return .none }
                state.marking = true
                state.keepFailure = nil
                let other = id == state.left.id ? state.right : state.left
                return .run { send in
                    var marked = [try await daemon.update(id, SessionUpdate(keeper: true))]
                    if other.keeper {
                        marked.append(try await daemon.update(other.id, SessionUpdate(keeper: false)))
                    }
                    await send(.marked(marked))
                } catch: { error, send in
                    await send(.markFailed(error.localizedDescription))
                }

            case .marked(let sessions):
                state.marking = false
                state.keepFailure = nil
                for session in sessions {
                    if session.id == state.left.id { state.left = session }
                    if session.id == state.right.id { state.right = session }
                }
                return .merge(sessions.map { .send(.delegate(.updated($0))) })
            }
        }
    }
}
