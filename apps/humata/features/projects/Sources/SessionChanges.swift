import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// Renames a session.
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
        package var leftChanges: [RepositoryChanges]?
        package var rightChanges: [RepositoryChanges]?
        /// Why the changes couldn't be read, and what to do about it.
        package var failure: String?
        package var marking = false

        package init(left: Session, right: Session) {
            self.left = left
            self.right = right
        }
    }

    package enum Action {
        /// The sheet appeared; reads both sessions' changes.
        case task
        case loaded(left: [RepositoryChanges], right: [RepositoryChanges])
        case loadFailed(String)
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
                    await send(.loaded(left: try await leftChanges, right: try await rightChanges))
                } catch: { error, send in
                    await send(.loadFailed(error.localizedDescription))
                }

            case .loaded(let left, let right):
                state.leftChanges = left
                state.rightChanges = right
                return .none

            case .loadFailed(let message), .markFailed(let message):
                state.marking = false
                state.failure = message
                return .none

            case .keepTapped(let id):
                guard !state.marking else { return .none }
                state.marking = true
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
                for session in sessions {
                    if session.id == state.left.id { state.left = session }
                    if session.id == state.right.id { state.right = session }
                }
                return .merge(sessions.map { .send(.delegate(.updated($0))) })
            }
        }
    }
}
