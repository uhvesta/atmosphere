import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// Settings › Repositories: the repositories offered when making or
/// changing a project, to rename or remove, and the GitHub users and
/// organizations whose repositories are among them, to add, remove and
/// list again. None of it changes a project, which keeps its own copies.
@Reducer
package struct SavedRepositoriesFeature {
    @ObservableState
    package struct State: Equatable {
        /// Nil until first read; last used first, then by name.
        package var repositories: [SavedRepository]?
        /// Nil until first read; by login.
        package var owners: [RepositoryOwner]?
        package var loading = false
        /// The repository whose name is being typed, and the name.
        package var renaming: SavedRepository.ID?
        package var newName = ""
        /// The login typed to add.
        package var newOwner = ""
        package var addingOwner = false
        /// The owners being listed again; "" while every one is.
        package var refreshing: Set<String> = []
        /// Owners just added whose repositories are still being listed.
        package var listing: Set<String> = []
        /// Why something couldn't be read or changed, and what to do.
        package var failure: String?

        package init() {}

        package var canAddOwner: Bool {
            !addingOwner && !newOwner.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case loaded(repositories: [SavedRepository], owners: [RepositoryOwner])
        case repositoriesChanged([SavedRepository])
        case ownersChanged([RepositoryOwner])
        case renameTapped(SavedRepository.ID)
        case renameSubmitted
        case renameCancelled
        case removeTapped(SavedRepository.ID)
        case addOwnerTapped
        case ownerAdded([RepositoryOwner])
        case removeOwnerTapped(String)
        /// Nil lists every owner again.
        case refreshTapped(String?)
        case refreshed(String?, [RepositoryOwner])
        /// What failed, for the person, and the message the daemon gave.
        case failed(Failure, String)
    }

    package enum Failure: Equatable, Sendable {
        case load
        case rename
        case remove
        case addOwner
        case removeOwner
        case refresh(String?)
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.continuousClock) var clock

    /// How often an owner just added is looked at while it's listed, and
    /// for how long.
    static let listingPoll: Duration = .seconds(2)
    static let listingPolls = 30

    private enum CancelID { case listing }

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding:
                return .none

            case .task:
                state.loading = true
                state.failure = nil
                return .run { send in
                    async let repositories = daemon.savedRepositories()
                    async let owners = daemon.repositoryOwners()
                    await send(.loaded(repositories: try await repositories, owners: try await owners))
                } catch: { error, send in
                    await send(.failed(.load, error.localizedDescription))
                }

            case .loaded(let repositories, let owners):
                state.loading = false
                state.repositories = repositories
                state.owners = owners
                return .none

            case .repositoriesChanged(let repositories):
                state.repositories = repositories
                return .none

            case .ownersChanged(let owners):
                state.owners = owners
                state.listing = state.listing.filter { login in Self.pending(login, in: owners) }
                return state.listing.isEmpty ? .cancel(id: CancelID.listing) : .none

            case .renameTapped(let id):
                guard let repository = state.repositories?.first(where: { $0.id == id }) else { return .none }
                state.renaming = id
                state.newName = repository.name
                return .none

            case .renameCancelled:
                state.renaming = nil
                state.newName = ""
                return .none

            case .renameSubmitted:
                guard let id = state.renaming else { return .none }
                let name = state.newName.trimmingCharacters(in: .whitespacesAndNewlines)
                state.renaming = nil
                state.newName = ""
                guard !name.isEmpty, state.repositories?.first(where: { $0.id == id })?.name != name else {
                    return .none
                }
                state.failure = nil
                return .run { send in
                    await send(.repositoriesChanged(try await daemon.renameSavedRepository(id, name)))
                } catch: { error, send in
                    await send(.failed(.rename, error.localizedDescription))
                }

            case .removeTapped(let id):
                state.failure = nil
                return .run { send in
                    await send(.repositoriesChanged(try await daemon.removeSavedRepository(id)))
                } catch: { error, send in
                    await send(.failed(.remove, error.localizedDescription))
                }

            case .addOwnerTapped:
                guard state.canAddOwner else { return .none }
                let login = state.newOwner.trimmingCharacters(in: .whitespacesAndNewlines)
                state.addingOwner = true
                state.failure = nil
                return .run { send in
                    await send(.ownerAdded(try await daemon.addRepositoryOwner(login)))
                } catch: { error, send in
                    await send(.failed(.addOwner, error.localizedDescription))
                }

            case .ownerAdded(let owners):
                state.addingOwner = false
                state.newOwner = ""
                let before = Set(state.owners?.map(\.login) ?? [])
                state.owners = owners
                // The daemon lists a new owner's repositories in the
                // background; look again until it has, or says why not.
                let added = owners.map(\.login).filter { !before.contains($0) }
                let pending = added.filter { Self.pending($0, in: owners) }
                state.listing.formUnion(pending)
                guard !state.listing.isEmpty else { return reloadRepositories() }
                return .merge(reloadRepositories(), poll())

            case .removeOwnerTapped(let login):
                state.failure = nil
                state.listing.remove(login)
                return .run { send in
                    await send(.ownersChanged(try await daemon.removeRepositoryOwner(login)))
                    await send(.repositoriesChanged(try await daemon.savedRepositories()))
                } catch: { error, send in
                    await send(.failed(.removeOwner, error.localizedDescription))
                }

            case .refreshTapped(let login):
                let key = login ?? ""
                guard !state.refreshing.contains(key), !state.refreshing.contains("") else { return .none }
                state.refreshing.insert(key)
                state.failure = nil
                return .run { send in
                    await send(.refreshed(login, try await daemon.refreshRepositoryOwners(login)))
                } catch: { error, send in
                    await send(.failed(.refresh(login), error.localizedDescription))
                }

            case .refreshed(let login, let owners):
                state.refreshing.remove(login ?? "")
                state.owners = owners
                return reloadRepositories()

            case .failed(let failure, let message):
                state.failure = Self.message(failure, message)
                switch failure {
                case .load: state.loading = false
                case .addOwner: state.addingOwner = false
                case .refresh(let login): state.refreshing.remove(login ?? "")
                case .rename, .remove, .removeOwner: break
                }
                return .none
            }
        }
    }

    /// Added, and neither listed nor failed yet.
    private static func pending(_ login: String, in owners: [RepositoryOwner]) -> Bool {
        guard let owner = owners.first(where: { $0.login == login }) else { return false }
        return owner.refreshed == nil && owner.error == nil
    }

    private func reloadRepositories() -> Effect<Action> {
        .run { send in
            await send(.repositoriesChanged(try await daemon.savedRepositories()))
        } catch: { error, send in
            await send(.failed(.load, error.localizedDescription))
        }
    }

    /// Reads the owners again every few seconds, for a minute at most,
    /// while one just added is being listed.
    private func poll() -> Effect<Action> {
        .run { send in
            for _ in 0..<Self.listingPolls {
                try await clock.sleep(for: Self.listingPoll)
                await send(.ownersChanged(try await daemon.repositoryOwners()))
                await send(.repositoriesChanged(try await daemon.savedRepositories()))
            }
        } catch: { error, send in
            guard !(error is CancellationError) else { return }
            await send(.failed(.load, error.localizedDescription))
        }
        .cancellable(id: CancelID.listing, cancelInFlight: true)
    }

    /// What failed and what is left as it was, then the daemon's message,
    /// which says how to fix it.
    private static func message(_ failure: Failure, _ message: String) -> String {
        switch failure {
        case .load: String(localized: "Couldn't read the saved repositories: \(message)")
        case .rename: String(localized: "Couldn't rename the repository, so it keeps its name: \(message)")
        case .remove: String(localized: "Couldn't remove the repository, so it's still offered: \(message)")
        case .addOwner: String(localized: "Couldn't add the owner: \(message)")
        case .removeOwner:
            String(localized: "Couldn't remove the owner, so its repositories are still offered: \(message)")
        case .refresh(let login?):
            String(
                localized: "Couldn't list \(login)'s repositories again; what was listed before is kept: \(message)")
        case .refresh(nil):
            String(
                localized: "Couldn't list the owners' repositories again; what was listed before is kept: \(message)")
        }
    }
}
