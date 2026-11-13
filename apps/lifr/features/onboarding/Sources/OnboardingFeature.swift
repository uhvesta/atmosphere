import ComposableArchitecture
import Foundation
import LifrCore
import LifrLockClient
import LifrRemindersClient
import LifrStoreClient
import LifrSyncClient

extension SharedKey where Self == AppStorageKey<Bool>.Default {
    /// Whether this device went through onboarding, or found records that
    /// made it unnecessary.
    package static var hasOnboarded: Self {
        Self[.appStorage("lifrHasOnboarded"), default: false]
    }
}

/// The first launch on a device: what Avenues, Projects and tasks are, a
/// couple of Avenues to start with, and the optional permissions, each with
/// Not Now. Ends on Today with a task that teaches picking a status.
@Reducer
package struct OnboardingFeature {
    /// An Avenue offered on the second screen.
    package struct Suggestion: Hashable, Sendable, Identifiable {
        package var key: String
        package var name: String
        package var symbol: String
        package var color: String
        package var id: String { key }
    }

    package enum Step: Hashable, Sendable {
        case welcome, avenues, setup
        /// Sync brought in records from another device while this one was
        /// onboarding, so there's nothing to set up.
        case found
    }

    @ObservableState
    package struct State: Equatable {
        package var step = Step.welcome
        package var picked: Set<String> = []
        /// An Avenue the person named themselves.
        package var custom = ""
        package var notifications = ReminderPermission.notDetermined
        package var biometry = Biometry.faceID
        package var sync = SyncState()
        @Shared(.locksApp) package var locksApp
        package var isFinishing = false
        package var error: String?

        package init(sync: SyncState = SyncState()) {
            self.sync = sync
        }

        package static let suggestions: [Suggestion] = [
            Suggestion(
                key: "HOME", name: String(localized: "Home"), symbol: "house", color: "orange"),
            Suggestion(
                key: "WORK", name: String(localized: "Work"), symbol: "briefcase", color: "blue"),
            Suggestion(
                key: "HEALTH", name: String(localized: "Health"), symbol: "heart", color: "red"),
            Suggestion(
                key: "MONEY", name: String(localized: "Money"), symbol: "banknote", color: "green"),
            Suggestion(
                key: "FAMILY", name: String(localized: "Family"), symbol: "person.2", color: "pink"),
            Suggestion(
                key: "LEARN", name: String(localized: "Learning"), symbol: "book", color: "purple"),
        ]
    }

    package enum Action: BindableAction, Equatable {
        case binding(BindingAction<State>)
        case task
        case continueTapped
        case backTapped
        case skipTapped
        case suggestionToggled(String)
        case notificationsLoaded(ReminderPermission)
        case allowNotificationsTapped
        case lockTapped
        case lockAnswered(Bool)
        case finished(Result<Snapshot, Failure>)
        case foundExisting
        case syncChanged(SyncState)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            /// Done; show Today. With a snapshot when onboarding saved
            /// records.
            case finished(Snapshot?)
        }
    }

    /// Why the Avenues couldn't be saved.
    package struct Failure: Error, Equatable {
        package var reason: String
    }

    @Dependency(\.store) private var store
    @Dependency(\.reminders) private var reminders
    @Dependency(\.lock) private var lock

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .task:
                state.biometry = lock.biometry()
                return .run { [reminders] send in await send(.notificationsLoaded(await reminders.permission())) }

            case .continueTapped:
                switch state.step {
                case .welcome: state.step = .avenues
                case .avenues: state.step = .setup
                case .setup: return finish(&state)
                case .found: return .send(.delegate(.finished(nil)))
                }
                return .none

            case .backTapped:
                switch state.step {
                case .avenues: state.step = .welcome
                case .setup: state.step = .avenues
                case .welcome, .found: break
                }
                return .none

            case .skipTapped:
                // Only the Inbox, which is already there.
                return .send(.delegate(.finished(nil)))

            case .suggestionToggled(let key):
                if state.picked.remove(key) == nil { state.picked.insert(key) }
                return .none

            case .notificationsLoaded(let permission):
                state.notifications = permission
                return .none

            case .allowNotificationsTapped:
                return .run { [reminders] send in
                    _ = try? await reminders.requestPermission()
                    await send(.notificationsLoaded(await reminders.permission()))
                }

            case .lockTapped:
                return .run { [lock] send in
                    do {
                        try await lock.authenticate(String(localized: "Lock Lifr when it opens"))
                        await send(.lockAnswered(true))
                    } catch {
                        await send(.lockAnswered(false))
                    }
                }

            case .lockAnswered(let on):
                if on { state.$locksApp.withLock { $0 = true } }
                return .none

            case .finished(.success(let snapshot)):
                state.isFinishing = false
                return .send(.delegate(.finished(snapshot)))

            case .finished(.failure(let failure)):
                state.isFinishing = false
                state.error = failure.reason
                return .none

            case .foundExisting:
                guard !state.isFinishing else { return .none }
                state.step = .found
                return .none

            case .syncChanged(let sync):
                state.sync = sync
                return .none

            case .binding, .delegate:
                return .none
            }
        }
    }

    /// Saves the picked Avenues and the starter task, in one change that
    /// syncs and can be undone.
    private func finish(_ state: inout State) -> Effect<Action> {
        guard !state.isFinishing else { return .none }
        state.isFinishing = true
        state.error = nil
        var picked = State.suggestions.filter { state.picked.contains($0.key) }
        let custom = state.custom.trimmingCharacters(in: .whitespacesAndNewlines)
        if !custom.isEmpty {
            picked.append(Suggestion(key: Self.key(for: custom), name: custom, symbol: "circle", color: "teal"))
        }
        let starter = String(localized: "Tap the circle to pick a status")
        return .run { [store, picked] send in
            do {
                let saved = try await store.save { (workspace, context) throws(LifrError) in
                    _ = workspace.ensureInbox(in: context)
                    var taken = Set(workspace.avenues.values.map(\.key.rawValue))
                    for suggestion in picked {
                        var key = suggestion.key
                        // A second Avenue with the same letters gets more of
                        // them, as a key can't repeat.
                        while taken.contains(key), key.count < 10 { key += "X" }
                        guard !taken.contains(key) else { continue }
                        taken.insert(key)
                        _ = try workspace.createAvenue(
                            key: key, name: suggestion.name, color: suggestion.color, symbol: suggestion.symbol,
                            in: context)
                    }
                    var task = Workspace.NewTask(title: starter, avenueID: Avenue.inboxID)
                    task.dueDate = CalendarDate(context.now, in: context.timeZone)
                    return try workspace.createTask(task, in: context)
                }
                await send(.finished(.success(saved.snapshot)))
            } catch {
                await send(.finished(.failure(Failure(reason: String(describing: error)))))
            }
        }
    }

    /// A key for an Avenue named `name`: its letters, capitalized, at most
    /// ten; "MINE" when it has fewer than two.
    package static func key(for name: String) -> String {
        let letters = name.folding(options: [.diacriticInsensitive, .caseInsensitive], locale: nil)
            .uppercased()
            .filter { ("A"..."Z").contains($0) }
        return letters.count >= 2 ? String(letters.prefix(10)) : "MINE"
    }
}
