import Foundation

extension InMemoryDaemon {
    /// What GitHub answers when an owner's repositories are listed.
    package enum OwnerListing: Hashable, Sendable {
        /// Its repositories, by name.
        case repositories([String])
        /// The listing fails, and the owner records why.
        case fails(String)
    }

    /// Puts these in the saved list as they are; ids are theirs.
    package func saveRepositories(_ repositories: [SavedRepository]) {
        state.withValue { state in
            state.savedRepositories = repositories
            state.nextSavedRepository = max(state.nextSavedRepository, repositories.map(\.id).max() ?? 0)
        }
    }

    /// Has GitHub answer `listing` the next times `login` is listed.
    package func listOnGitHub(_ login: String, _ listing: OwnerListing) {
        state.withValue { $0.ownerListings[login.lowercased()] = listing }
    }

    /// Last used first, then by name, as the daemon lists them.
    func savedRepositories() -> [SavedRepository] {
        state.value.savedRepositories.sorted { left, right in
            switch (left.lastUsed, right.lastUsed) {
            case (let l?, let r?) where l != r: l > r
            case (.some, nil): true
            case (nil, .some): false
            default: left.name.localizedStandardCompare(right.name) == .orderedAscending
            }
        }
    }

    func renameSavedRepository(_ id: Int64, _ name: String) throws -> [SavedRepository] {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else {
            throw DaemonError.refused(.invalidRequest, message: "Give the repository a name; it wasn't renamed.")
        }
        try state.withValue { state in
            guard let index = state.savedRepositories.firstIndex(where: { $0.id == id }) else {
                throw Self.unknownSaved(id)
            }
            state.savedRepositories[index].name = name
        }
        return savedRepositories()
    }

    func removeSavedRepository(_ id: Int64) throws -> [SavedRepository] {
        try state.withValue { state in
            guard let index = state.savedRepositories.firstIndex(where: { $0.id == id }) else {
                throw Self.unknownSaved(id)
            }
            state.savedRepositories.remove(at: index)
        }
        return savedRepositories()
    }

    func repositoryOwners() -> [RepositoryOwner] {
        state.value.repositoryOwners.sorted { $0.login < $1.login }
    }

    func addRepositoryOwner(_ given: String) throws -> [RepositoryOwner] {
        let login = try Self.login(given)
        let added = state.withValue { state in
            guard !state.repositoryOwners.contains(where: { $0.login == login }) else { return false }
            state.repositoryOwners.append(RepositoryOwner(login: login, added: start))
            return true
        }
        if added { list(login) }
        return repositoryOwners()
    }

    func removeRepositoryOwner(_ login: String) throws -> [RepositoryOwner] {
        let login = login.lowercased()
        try state.withValue { state in
            guard state.repositoryOwners.contains(where: { $0.login == login }) else {
                throw DaemonError.refused(
                    .invalidRequest, message: "\(login) isn't a saved owner; list the owners to see which are.")
            }
            state.repositoryOwners.removeAll { $0.login == login }
            state.savedRepositories.removeAll { $0.source == .github && $0.owner == login }
        }
        return repositoryOwners()
    }

    func refreshRepositoryOwners(_ login: String?) throws -> [RepositoryOwner] {
        let logins = try state.withValue { state in
            guard let login = login?.lowercased() else { return state.repositoryOwners.map(\.login) }
            guard state.repositoryOwners.contains(where: { $0.login == login }) else {
                throw DaemonError.refused(
                    .invalidRequest, message: "\(login) isn't a saved owner; add it first, then refresh it.")
            }
            return [login]
        }
        for login in logins { list(login) }
        return repositoryOwners()
    }

    /// Lists `login`'s repositories as GitHub is set to answer, keeping
    /// what was listed before when it fails.
    private func list(_ login: String) {
        state.withValue { state in
            guard let owner = state.repositoryOwners.firstIndex(where: { $0.login == login }) else { return }
            switch state.ownerListings[login] ?? .repositories([]) {
            case .fails(let why):
                state.repositoryOwners[owner].error = why
            case .repositories(let names):
                for name in names {
                    let location = "https://github.com/\(login)/\(name)"
                    guard !state.savedRepositories.contains(where: { $0.location == location }) else { continue }
                    state.nextSavedRepository += 1
                    state.savedRepositories.append(
                        SavedRepository(
                            id: state.nextSavedRepository, name: name, location: location, source: .github,
                            owner: login, added: start))
                }
                state.repositoryOwners[owner].error = nil
                state.repositoryOwners[owner].refreshed = start
            }
            state.repositoryOwners[owner].repositories =
                state.savedRepositories.filter { $0.source == .github && $0.owner == login }.count
        }
    }

    /// The login in `octo-org`, `@octo-org` or `https://github.com/octo-org`.
    private static func login(_ given: String) throws -> String {
        var login = given.trimmingCharacters(in: .whitespacesAndNewlines)
        for prefix in ["https://github.com/", "http://github.com/", "github.com/", "@"] where login.hasPrefix(prefix) {
            login.removeFirst(prefix.count)
        }
        while login.hasSuffix("/") { login.removeLast() }
        guard !login.isEmpty, login.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "-" }) else {
            throw DaemonError.refused(
                .invalidRequest,
                message:
                    "\(given) isn't a GitHub login; give one as octo-org, @octo-org or https://github.com/octo-org.")
        }
        return login.lowercased()
    }

    private static func unknownSaved(_ id: Int64) -> DaemonError {
        .refused(.invalidRequest, message: "There is no saved repository \(id); list them to find its id.")
    }
}
