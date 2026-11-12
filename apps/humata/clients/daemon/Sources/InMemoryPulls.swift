import Foundation

extension InMemoryDaemon {
    /// An open issue on GitHub, to import as a task.
    package struct Issue: Hashable, Sendable {
        package var number: UInt64
        package var title: String
        package var body: String

        package init(number: UInt64, title: String, body: String = "") {
            self.number = number
            self.title = title
            self.body = body
        }
    }

    /// Gives `project` these open issues on GitHub.
    package func openIssues(_ issues: [Issue], in project: String) {
        state.withValue { $0.issues[project] = issues }
    }

    /// As if GitHub said the session's pull requests merged: its task is
    /// done and the session archived.
    package func mergePullRequests(of session: String) throws {
        let item = try state.withValue { state in
            guard let index = state.sessions.firstIndex(where: { $0.id == session }) else {
                throw Self.unknown(session)
            }
            for pull in state.pullRequests.indices where state.pullRequests[pull].sessionID == session {
                state.pullRequests[pull].state = .merged
                state.pullRequests[pull].review = .approved
                state.pullRequests[pull].checks = .passing
            }
            state.sessions[index].archived = true
            return state.sessions[index].itemID
        }
        if let item { _ = try updateItem(item, ItemUpdate(status: .done)) }
    }

    /// Each of the project's repositories lands; with pull requests, each
    /// opens one on `acme/<repository>`, or keeps the one it opened.
    func land(_ session: String, _ request: LandRequest) throws -> Landed {
        let branch = request.branch.trimmingCharacters(in: .whitespaces)
        guard !branch.isEmpty, !branch.contains(" "), !branch.hasPrefix("-") else {
            throw DaemonError.refused(
                .invalidRequest, message: "\(branch.isEmpty ? "A branch" : branch) needs a name git takes: no spaces.")
        }
        let title = request.message.split(separator: "\n").first.map(String.init) ?? ""
        guard !title.trimmingCharacters(in: .whitespaces).isEmpty else {
            throw DaemonError.refused(.invalidRequest, message: "Write a commit message to land with.")
        }
        return try state.withValue { state in
            guard let found = state.sessions.first(where: { $0.id == session }) else { throw Self.unknown(session) }
            let names = state.projects.first { $0.id == found.projectID }?.repositories.map(\.name) ?? []
            guard !names.isEmpty else {
                throw DaemonError.refused(.invalidRequest, message: "Only a session in a project has code to land.")
            }
            let push = request.push || request.openPullRequests
            var landed = Landed(repositories: [])
            for (index, name) in names.enumerated() {
                landed.repositories.append(
                    LandedRepository(
                        name: name, branch: branch, commit: String(format: "%07x", 0xabc_0000 + index), base: "main",
                        originURL: "git@github.com:acme/\(name).git", pushed: push))
                guard request.openPullRequests else { continue }
                if let kept = state.pullRequests.first(where: {
                    $0.sessionID == session && $0.repository == name && $0.state == .open
                }) {
                    landed.pullRequests.append(kept)
                    continue
                }
                let number = UInt64(state.pullRequests.count + 12)
                let pull = PullRequest(
                    sessionID: session, itemID: found.itemID, repository: name, githubRepository: "acme/\(name)",
                    number: number, url: "https://github.com/acme/\(name)/pull/\(number)", title: title,
                    branch: branch, base: "main", checks: .pending, read: start + Double(2000 + number))
                state.pullRequests.append(pull)
                landed.pullRequests.append(pull)
            }
            return landed
        }
    }

    func pullRequests(_ of: PullRequestsOf) -> [PullRequest] {
        state.value.pullRequests.filter { pull in
            switch of {
            case .session(let session): pull.sessionID == session
            case .item(let item): pull.itemID == item
            }
        }
    }

    /// Every repository is up to date.
    func updateFromBase(_ session: String) throws -> BaseUpdate {
        let names = try state.withValue { state in
            guard let found = state.sessions.first(where: { $0.id == session }) else { throw Self.unknown(session) }
            return state.projects.first { $0.id == found.projectID }?.repositories.map(\.name) ?? []
        }
        return BaseUpdate(
            repositories: names.map { MergedRepository(name: $0, upstream: "origin/main", outcome: .upToDate) })
    }

    func importIssues(_ project: String) throws -> ImportedIssues {
        let (issues, skipped) = try state.withValue { state in
            guard state.projects.contains(where: { $0.id == project }) else {
                throw DaemonError.refused(.unknownProject, message: "There is no project \(project).")
            }
            let all = state.issues[project] ?? []
            let new = all.filter { !state.importedIssues.contains("\(project)#\($0.number)") }
            for issue in new { state.importedIssues.insert("\(project)#\(issue.number)") }
            return (new, all.count - new.count)
        }
        let items = try issues.map { issue in
            try createItem(project, ItemFields(title: issue.title, spec: issue.body))
        }
        return ImportedIssues(items: items, skipped: skipped)
    }
}
