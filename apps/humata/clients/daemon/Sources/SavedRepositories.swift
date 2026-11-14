import Foundation

/// A repository to pick when making or changing a project: one a project
/// was made or changed with, or one listed on GitHub for an owner. A
/// project keeps its own copy of what was picked, so renaming or removing
/// one never changes a project.
package struct SavedRepository: Identifiable, Hashable, Sendable {
    package enum Source: Hashable, Sendable {
        /// A project was made or changed with it.
        case used
        /// It was listed on GitHub for `owner`.
        case github
    }

    package var id: Int64
    /// What it's called in the list.
    package var name: String
    /// A folder on the Mac, or a URL to clone.
    package var location: String
    package var source: Source
    /// The GitHub login it was listed for; empty for one a project used.
    package var owner: String
    package var added: Date
    /// When a project was last made or changed with it; nil if never.
    package var lastUsed: Date?

    package init(
        id: Int64, name: String, location: String, source: Source = .used, owner: String = "",
        added: Date = Date(timeIntervalSince1970: 0), lastUsed: Date? = nil
    ) {
        self.id = id
        self.name = name
        self.location = location
        self.source = source
        self.owner = owner
        self.added = added
        self.lastUsed = lastUsed
    }
}

/// A GitHub user or organization whose repositories are saved to pick from.
package struct RepositoryOwner: Identifiable, Hashable, Sendable {
    package var login: String
    package var added: Date
    /// When its repositories were last listed; nil if never.
    package var refreshed: Date?
    /// Why the last listing failed, and how to fix it; nil if it didn't.
    package var error: String?
    /// How many of its repositories are saved and shown.
    package var repositories: Int

    package var id: String { login }

    package init(
        login: String, added: Date = Date(timeIntervalSince1970: 0), refreshed: Date? = nil, error: String? = nil,
        repositories: Int = 0
    ) {
        self.login = login
        self.added = added
        self.refreshed = refreshed
        self.error = error
        self.repositories = repositories
    }
}
