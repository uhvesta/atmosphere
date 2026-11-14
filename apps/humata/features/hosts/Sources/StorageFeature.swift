import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// Settings › Storage: what Humata's files take on this Mac, by kind, each
/// shown in Finder; cleaning up by hand, each part saying what it removes
/// and how much first; and the daily cleanup, off unless the person turns
/// it on. A cleanup runs in the daemon, and its progress is read again
/// every half second until it's done; a read that fails is made again for
/// about a minute before the cleanup is given up as lost.
@Reducer
package struct StorageFeature {
    @ObservableState
    package struct State: Equatable {
        /// Nil until first read, while the folders are measured.
        package var storage: Storage?
        /// The models downloaded for the model server, largest first.
        package var models: [ModelFiles.Model] = []
        /// Where the models are, to show in Finder.
        package var modelsFolder = ""
        /// The model being deleted, shown with a spinner.
        package var deletingModel: String?
        /// Why it couldn't be read, and what to do about it; gone with the
        /// next read that works.
        package var failure: String?
        /// Why what the person asked for wasn't done: a setting not saved,
        /// a model not deleted, a cleanup not started. It stays through a
        /// cleanup's progress reads, until they next change or confirm
        /// something.
        package var actionFailure: String?
        @Presents package var alert: AlertState<Action.Alert>?

        package init(storage: Storage? = nil) {
            self.storage = storage
        }

        /// How long archived sessions are kept, as offered.
        package static let dayChoices = [30, 45, 90, 180]

        package var modelsBytes: UInt64 { models.reduce(0) { $0 + $1.bytes } }

        /// humatad's files and the models together.
        package var total: UInt64 { (storage?.total ?? 0) + modelsBytes }
    }

    /// How many reads of a cleanup's progress may fail in a row, two seconds
    /// apart, before it's given up: about a minute.
    static let progressTries = 30

    package enum Action {
        case task
        case loaded(Storage)
        case modelsLoaded([ModelFiles.Model])
        case failed(String)
        /// The settings weren't saved; the switch and days go back to these.
        case settingsFailed(autoDelete: Bool, days: Int, String)
        /// The cleanup didn't start.
        case cleanupFailed(String)
        /// A cleanup's progress couldn't be read for too long to keep
        /// waiting on: it's shown no more, and may still be running.
        case cleanupLost(String)
        case modelDeleteFailed(String)
        case autoDeleteToggled(Bool)
        case daysPicked(Int)
        /// Each asks before removing anything.
        case deleteArchivedTapped
        case putAwayWorktreesTapped
        case removeUnusedClonesTapped
        case deleteModelTapped(String)
        case compactTapped
        case showInFinderTapped(String)
        case alert(PresentationAction<Alert>)

        package enum Alert: Equatable {
            case deleteArchived(days: Int)
            case putAwayWorktrees
            case removeUnusedClones
            case deleteModel(id: String)
        }
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.modelFiles) var modelFiles
    @Dependency(\.continuousClock) var clock
    @Dependency(\.openURL) var openURL

    private enum CancelID { case progress }

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                state.modelsFolder = modelFiles.folder
                return .merge(
                    .run { send in
                        await send(.loaded(try await daemon.storage()))
                    } catch: { error, send in
                        await send(.failed(error.localizedDescription))
                    },
                    loadModels())

            case .loaded(let storage):
                state.failure = nil
                state.storage = storage
                guard storage.job != nil else { return .cancel(id: CancelID.progress) }
                return .run { send in
                    var failures = 0
                    while true {
                        // Only the sleep throws here, and only when cancelled.
                        try await clock.sleep(for: failures == 0 ? .milliseconds(500) : .seconds(2))
                        do {
                            // The next read is `.loaded`'s to schedule.
                            await send(.loaded(try await daemon.storage()))
                            return
                        } catch is CancellationError {
                            return
                        } catch {
                            failures += 1
                            guard failures < Self.progressTries else {
                                await send(.cleanupLost(error.localizedDescription))
                                return
                            }
                            // Shown while it's read again: the cleanup may
                            // well be running still.
                            await send(.failed(error.localizedDescription))
                        }
                    }
                }
                .cancellable(id: CancelID.progress, cancelInFlight: true)

            case .modelsLoaded(let models):
                state.models = models.sorted { $0.bytes > $1.bytes }
                state.deletingModel = nil
                return .none

            case .failed(let message):
                state.failure = message
                return .none

            case .cleanupFailed(let message):
                state.actionFailure = message
                state.storage?.job = nil
                return .none

            case .cleanupLost(let message):
                state.failure = message
                state.storage?.job = nil
                return .none

            case .modelDeleteFailed(let message):
                state.actionFailure = message
                state.deletingModel = nil
                return .none

            case .settingsFailed(let autoDelete, let days, let message):
                state.actionFailure = message
                state.storage?.autoDelete = autoDelete
                state.storage?.days = days
                return .none

            case .autoDeleteToggled(let on):
                guard let storage = state.storage else { return .none }
                state.actionFailure = nil
                state.storage?.autoDelete = on
                return set(autoDelete: on, days: storage.days, was: storage)

            case .daysPicked(let days):
                guard let storage = state.storage else { return .none }
                state.actionFailure = nil
                state.storage?.days = days
                return set(autoDelete: storage.autoDelete, days: days, was: storage)

            case .deleteArchivedTapped:
                guard let days = state.storage?.days else { return .none }
                state.alert = confirm(
                    "Delete archived sessions not used in \(days) days?", button: "Delete",
                    action: .deleteArchived(days: days),
                    message: """
                        Their histories, snapshots and branches go for good; this can't be undone. Keepers, \
                        sessions with forks, and sessions with commits nothing else holds are kept.
                        """)
                return .none

            case .putAwayWorktreesTapped:
                guard let bytes = state.storage?.archivedWorktreesBytes else { return .none }
                state.alert = confirm(
                    "Remove archived sessions' working copies?", button: "Remove",
                    action: .putAwayWorktrees,
                    message: """
                        Frees about \(StorageView.size(bytes)). Uncommitted work in them is kept as a snapshot, \
                        and unarchiving a session makes its copy again.
                        """)
                return .none

            case .removeUnusedClonesTapped:
                guard let bytes = state.storage?.unusedClonesBytes else { return .none }
                state.alert = confirm(
                    "Delete the repositories no project uses?", button: "Delete",
                    action: .removeUnusedClones,
                    message: """
                        Frees about \(StorageView.size(bytes)). Their branches and snapshots go for good; this \
                        can't be undone. Adding the project again clones it anew.
                        """)
                return .none

            case .deleteModelTapped(let id):
                guard let model = state.models.first(where: { $0.id == id }) else { return .none }
                state.alert = confirm(
                    "Delete \(model.name)?", button: "Delete", action: .deleteModel(id: id),
                    message: """
                        Frees \(StorageView.size(model.bytes)). Sessions on it can't run until it's downloaded \
                        again, which can't be undone from here.
                        """)
                return .none

            case .alert(.presented(.deleteArchived(let days))):
                return cleanUp(&state, Storage.Cleanup(deleteAfterDays: days, compact: true))

            case .alert(.presented(.putAwayWorktrees)):
                return cleanUp(&state, Storage.Cleanup(putAwayWorktrees: true))

            case .alert(.presented(.removeUnusedClones)):
                return cleanUp(&state, Storage.Cleanup(removeUnusedClones: true))

            case .alert(.presented(.deleteModel(let id))):
                guard state.deletingModel == nil else { return .none }
                state.actionFailure = nil
                state.deletingModel = id
                return .run { send in
                    try await modelFiles.delete(id)
                    await send(.modelsLoaded(await modelFiles.installed()))
                } catch: { error, send in
                    await send(.modelDeleteFailed(error.localizedDescription))
                }

            case .alert:
                return .none

            case .compactTapped:
                return cleanUp(&state, Storage.Cleanup(compact: true))

            case .showInFinderTapped(let path):
                // The folder itself, or a file's folder, as Finder opens it.
                var url = URL(fileURLWithPath: path)
                if !FileManager.default.fileExists(atPath: path) || !url.hasDirectoryPath {
                    url.deleteLastPathComponent()
                }
                return .run { _ in await openURL(url) }
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }

    private func confirm(_ title: String, button: String, action: Action.Alert, message: String) -> AlertState<
        Action.Alert
    > {
        AlertState {
            TextState(title)
        } actions: {
            ButtonState(role: .destructive, action: action) { TextState(button) }
            ButtonState(role: .cancel) { TextState("Cancel") }
        } message: {
            TextState(message)
        }
    }

    private func loadModels() -> Effect<Action> {
        .run { send in await send(.modelsLoaded(await modelFiles.installed())) }
    }

    private func set(autoDelete: Bool, days: Int, was: Storage) -> Effect<Action> {
        .run { send in
            await send(.loaded(try await daemon.setStorageSettings(autoDelete, days)))
        } catch: { error, send in
            await send(.settingsFailed(autoDelete: was.autoDelete, days: was.days, error.localizedDescription))
        }
    }

    private func cleanUp(_ state: inout State, _ cleanup: Storage.Cleanup) -> Effect<Action> {
        guard state.storage?.job == nil else { return .none }
        state.actionFailure = nil
        // Shown at once, before the daemon's first word on it.
        state.storage?.job = Storage.Job(step: "Starting")
        return .run { send in
            await send(.loaded(try await daemon.cleanUpStorage(cleanup)))
        } catch: { error, send in
            await send(.cleanupFailed(error.localizedDescription))
        }
    }
}
