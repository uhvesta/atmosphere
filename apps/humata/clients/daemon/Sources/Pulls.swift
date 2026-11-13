import Foundation

/// How to land a session's work: committed onto `branch` in each repository
/// it changed, and pushed or opened as pull requests on GitHub.
package struct LandRequest: Hashable, Sendable {
    package var branch: String
    /// The commit message; its first line titles the pull requests.
    package var message: String
    package var push: Bool
    /// Push, and open or find a pull request for each branch.
    package var openPullRequests: Bool

    package init(branch: String, message: String, push: Bool = false, openPullRequests: Bool = false) {
        self.branch = branch
        self.message = message
        self.push = push
        self.openPullRequests = openPullRequests
    }
}

/// One repository's landed branch.
package struct LandedRepository: Hashable, Sendable {
    package var name: String
    package var branch: String
    /// Empty if it had no changes since it last landed.
    package var commit: String
    /// The branch its work goes into.
    package var base: String
    package var originURL: String
    package var pushed: Bool

    package init(
        name: String, branch: String, commit: String, base: String, originURL: String = "", pushed: Bool = false
    ) {
        self.name = name
        self.branch = branch
        self.commit = commit
        self.base = base
        self.originURL = originURL
        self.pushed = pushed
    }
}

package struct Landed: Hashable, Sendable {
    package var repositories: [LandedRepository]
    package var pullRequests: [PullRequest]

    package init(repositories: [LandedRepository], pullRequests: [PullRequest] = []) {
        self.repositories = repositories
        self.pullRequests = pullRequests
    }
}

/// How this Mac follows open pull requests on GitHub. Refreshing a task's
/// reads them either way.
package struct PullRefresh: Equatable, Sendable {
    /// The fewest and most minutes between reads.
    package static let minutes = 1...1440
    /// What the Mac does until the person picks otherwise.
    package static let standard = PullRefresh(automatic: true, everyMinutes: 2)

    /// Read on their own every `everyMinutes`, or only when refreshed.
    package var automatic: Bool
    package var everyMinutes: Int

    package init(automatic: Bool, everyMinutes: Int) {
        self.automatic = automatic
        self.everyMinutes = everyMinutes
    }
}

/// A pull request a session opened, as last read from GitHub.
package struct PullRequest: Identifiable, Hashable, Sendable {
    package enum State: Hashable, Sendable {
        case open
        case merged
        case closed
    }

    /// Where the reviewers stand together.
    package enum Review: Hashable, Sendable {
        case none
        case changesRequested
        case commented
        case approved
    }

    package enum Checks: Hashable, Sendable {
        case none
        case pending
        case passing
        case failing
    }

    package var id: String { "\(githubRepository)#\(number)" }
    package var sessionID: String
    package var itemID: String?
    /// The session's name for the repository.
    package var repository: String
    /// On GitHub, as `owner/name`.
    package var githubRepository: String
    package var number: UInt64
    package var url: String
    package var title: String
    package var branch: String
    package var base: String
    package var state: State
    package var review: Review
    package var checks: Checks
    package var read: Date
    /// Why it couldn't be read from GitHub last time; nil once it was.
    /// What's shown is as it was last read.
    package var problem: String?

    package init(
        sessionID: String, itemID: String? = nil, repository: String, githubRepository: String, number: UInt64,
        url: String, title: String, branch: String, base: String, state: State = .open, review: Review = .none,
        checks: Checks = .none, read: Date, problem: String? = nil
    ) {
        self.problem = problem
        self.sessionID = sessionID
        self.itemID = itemID
        self.repository = repository
        self.githubRepository = githubRepository
        self.number = number
        self.url = url
        self.title = title
        self.branch = branch
        self.base = base
        self.state = state
        self.review = review
        self.checks = checks
        self.read = read
    }
}

/// Whose pull requests to list.
package enum PullRequestsOf: Hashable, Sendable {
    case session(String)
    /// Every session's of the task.
    case item(String)
}

/// How merging its base went for one repository.
package struct MergedRepository: Hashable, Sendable {
    package enum Outcome: Hashable, Sendable {
        case upToDate
        case merged
        case conflicts
    }

    package var name: String
    /// The branch merged in, as `origin/main`.
    package var upstream: String
    package var outcome: Outcome
    /// The files left with conflicts.
    package var conflicts: [String]

    package init(name: String, upstream: String, outcome: Outcome, conflicts: [String] = []) {
        self.name = name
        self.upstream = upstream
        self.outcome = outcome
        self.conflicts = conflicts
    }
}

package struct BaseUpdate: Hashable, Sendable {
    package var repositories: [MergedRepository]
    /// The agent was asked to resolve the conflicts.
    package var sentToAgent: Bool

    package init(repositories: [MergedRepository], sentToAgent: Bool = false) {
        self.repositories = repositories
        self.sentToAgent = sentToAgent
    }
}

package struct ImportedIssues: Hashable, Sendable {
    /// The tasks made, one per issue.
    package var items: [Item]
    /// How many were imported before.
    package var skipped: Int

    package init(items: [Item], skipped: Int = 0) {
        self.items = items
        self.skipped = skipped
    }
}
