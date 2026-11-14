import ComposableArchitecture
import Foundation

/// A daemon kept in memory, for tests and previews. Each message plays the
/// steps `script(text)` gives, at once; a step that needs permission waits
/// for an answer unless the session's approval mode allows it. A session in
/// a project writes to its own copy of the code, which a fork with code
/// copies as it was at the fork and a chat-only fork shares. Ids count up
/// (`p1`, `s1`, `e1`, `r1`, ...) and times from `start`, so tests see the
/// same values every run.
package final class InMemoryDaemon: Sendable {
    /// What the agent does in reply to a message.
    package enum Step: Hashable, Sendable {
        case say(String)
        case think(String)
        /// Asks to run `command`; if allowed, reports `output`.
        case run(command: String, output: String)
        /// Asks to write `contents` to `path`; if allowed, writes it to the
        /// session's ``files(of:)``.
        case write(path: String, contents: String)
        /// Proposes a task, or a subtask under `parentID`, with Humata's
        /// propose_task tool; nothing is saved until it's applied.
        case propose(title: String, spec: String, parentID: String? = nil)
        /// Proposes `spec` as `itemID`'s new spec.
        case proposeSpecEdit(itemID: String, spec: String)
    }

    /// A turn stopped at a permission request, with the steps after it.
    struct Paused {
        var request: String
        var step: Step
        var callID: String
        var rest: [Step]
    }

    /// A file written in a copy, and the event it came after: the agent's
    /// writes and the person's reverts. Nil contents removed it.
    struct Write: Hashable {
        var event: String
        var path: String
        var contents: String?
    }

    struct Queued {
        var id: String
        var text: String
        var images: [String] = []
    }

    struct State {
        var projects: [Project] = []
        var sessions: [Session] = []
        var histories: [String: [SessionEvent]] = [:]
        var paused: [String: Paused] = [:]
        var queues: [String: [Queued]] = [:]
        /// Each session's histories before its rewinds, the last last.
        var rewinds: [String: [[SessionEvent]]] = [:]
        var skills = SkillList()
        var plugins = PluginList()
        var sessionDefaults = SessionDefaults()
        var nextQueued = 1
        /// Images sent to each session, by session id then name.
        var images: [String: [String: ImageAttachment]] = [:]
        /// Each copy of the code's writes, oldest first, by the id of the
        /// session that made the copy.
        var copies: [String: [Write]] = [:]
        /// The copy each session works in, by session id.
        var copyOf: [String: String] = [:]
        var watchers: [String: [UUID: AsyncThrowingStream<WatchItem, any Error>.Continuation]] = [:]
        /// Items as given, in the order they were added.
        var items: [Item] = []
        /// Board changes, oldest first.
        var changes: [BoardChange] = []
        /// Removed items, so reverting the removal puts them back.
        var removed: [String: Item] = [:]
        var hosts: [RemoteHost] = []
        /// Each host's variables, by host.
        var env: [String: [String: String]] = [:]
        /// The hosts Humata's program was installed on.
        var installed: Set<String> = []
        /// The hosts that can't be reached.
        var offline: Set<String> = []
        var nextProject = 1
        var nextItem = 1
        var nextChange: UInt64 = 1
        var nextSession = 1
        var nextEvent = 1
        var nextRequest = 1
        /// Each copy's staged files, by copy; nil contents staged a removal.
        var staged: [String: [String: String?]] = [:]
        /// The contents each session marked viewed, by file.
        var viewed: [String: [FileRef: String]] = [:]
        var threads: [Thread] = []
        var pullRequests: [PullRequest] = []
        var pullRefresh = PullRefresh.standard
        /// Each project's open issues on GitHub, by project.
        var issues: [String: [Issue]] = [:]
        /// The issues made tasks, as `project#number`.
        var importedIssues: Set<String> = []
        /// The sessions started outside Humata, by host; "" for this Mac.
        var importable: [String: [ImportableSession]] = [:]
        var skipped: [String: [SkippedSession]] = [:]
        /// The sessions their hosts lost, still in `sessions` for their copies.
        var lost: Set<String> = []
        /// Files on each host outside sessions' folders, by absolute path;
        /// "" for this Mac.
        var hostFiles: [String: [String: Data]] = [:]
        /// What each backend offers; a backend missing here can't list them.
        var models: [Backend: [ModelChoice]] = [
            .claude: [
                ModelChoice(id: "opus", name: "Opus", detail: "The most capable"),
                ModelChoice(id: "sonnet", name: "Sonnet", detail: "Fast and capable"),
            ],
            .copilot: [
                ModelChoice(id: "gpt-5", name: "GPT-5", detail: "1 premium request"),
                ModelChoice(id: "claude-sonnet-4.5", name: "Claude Sonnet 4.5", detail: "1 premium request"),
            ],
            .local: [ModelChoice(id: "qwen3:8b", name: "qwen3:8b", readsImages: false)],
        ]
        /// The sessions imported, by the backend's id.
        var imported: [String: String] = [:]
        var reviews: [Review] = []
        var nextThread = 1
        var nextComment = 1
        /// Agents' requests not answered yet, oldest first.
        var requests: [AgentRequest] = []
        var nextAgentRequest = 1
        /// The open terminals, by id.
        var terminals: [String: FakeTerminal] = [:]
        var nextTerminal = 1
        /// The ports being forwarded, oldest first.
        var forwards: [Forward] = []
        var nextForward = 1
        /// The ports that answer, on each host; "" is this Mac.
        var listening: [String: Set<Int>] = [:]
        /// What the person did about each session's proposals, by session
        /// then proposal.
        var decisions: [String: [String: Proposal.State]] = [:]
        /// What outside agents proposed, by project, oldest first, as
        /// proposed; decided in `decisions` with no session.
        var outside: [String: [Proposal]] = [:]
        /// Admin actions other clients asked for, oldest first.
        var approvals: [AdminApproval] = []
        /// Grants in force.
        var grants: [ApprovalGrant] = []
        var storage = Storage(
            uses: [
                .init(
                    kind: .clones, bytes: 503_000_000,
                    path: "/Users/you/Library/Application Support/Humata/repositories"),
                .init(kind: .worktrees, bytes: 412_000_000, path: "/Users/you/Library/Caches/Humata/worktrees"),
                .init(kind: .histories, bytes: 61_000_000, path: "/Users/you/Library/Application Support/Humata/host"),
                .init(
                    kind: .recovery, bytes: 18_000_000, path: "/Users/you/Library/Application Support/Humata/recovery"),
                .init(
                    kind: .store, bytes: 9_400_000, path: "/Users/you/Library/Application Support/Humata/humata.sqlite3"
                ),
                .init(
                    kind: .search, bytes: 3_100_000,
                    path: "/Users/you/Library/Application Support/Humata/humata.sqlite3"),
                .init(kind: .logs, bytes: 1_200_000, path: "/Users/you/Library/Application Support/Humata/host.log"),
            ], archivedWorktreesBytes: 126_000_000, unusedClonesBytes: 88_000_000)
        var nextGrant = 1
        var commandLinks = CommandLinks(
            folder: "/Users/you/.local/bin",
            links: [.init(name: "humata", state: .missing), .init(name: "humatad", state: .missing)],
            onPath: false)
    }

    let state = LockIsolated(State())
    private let script: @Sendable (String) -> [Step]
    package let start: Date
    private let sshConfigHosts: [String]

    /// `sshHosts` are the hosts its ~/.ssh/config names.
    package init(
        start: Date = Date(timeIntervalSince1970: 1_794_000_000), sshHosts: [String] = [],
        script: @escaping @Sendable (String) -> [Step] = { [.say("You said: \($0)")] }
    ) {
        self.start = start
        self.sshConfigHosts = sshHosts
        self.script = script
    }

    /// Has something answer on `port` on `host`, or this Mac if nil, so it
    /// can be forwarded.
    package func listen(on port: Int, at host: String? = nil) {
        state.withValue { state in _ = state.listening[host ?? "", default: []].insert(port) }
    }

    private func forwardPort(_ host: String?, _ port: Int, _ session: String?) throws -> Forward {
        try state.withValue { state in
            if let host, state.offline.contains(host) {
                throw DaemonError.refused(
                    .hostUnavailable, message: "Couldn't reach \(host). Check that it is up, then try again.")
            }
            guard state.listening[host ?? ""]?.contains(port) == true else {
                throw DaemonError.refused(
                    .hostUnavailable, message: "Nothing answered on port \(port). Start the server, then try again.")
            }
            // The same port here, unless it's taken.
            let taken = Set(state.forwards.map(\.localPort)).union(state.listening[""] ?? [])
            let local = host == nil || taken.contains(port) ? 50_000 + state.nextForward : port
            let forward = Forward(
                id: "f\(state.nextForward)", host: host, port: port, localPort: local, sessionID: session)
            state.nextForward += 1
            state.forwards.append(forward)
            return forward
        }
    }

    private func stopForward(_ id: String) throws {
        try state.withValue { state in
            guard state.forwards.contains(where: { $0.id == id }) else {
                throw DaemonError.refused(.unknownForward, message: "There is no forward \(id); it was stopped.")
            }
            state.forwards.removeAll { $0.id == id }
        }
    }

    /// Makes `host` unreachable, as a dropped network would, or reachable
    /// again.
    package func setOffline(_ host: String, _ offline: Bool) {
        state.withValue { state in
            if offline { state.offline.insert(host) } else { state.offline.remove(host) }
        }
    }

    /// The variables set for `host`'s agents, values and all, which the
    /// client never sees.
    package func env(of host: String) -> [String: String] {
        state.value.env[host] ?? [:]
    }

    /// The sessions as they are now.
    package var sessions: [Session] { state.value.sessions }

    /// Has `session`'s agent ask a question or for a move, as its
    /// ask_person and request_move tools do.
    @discardableResult
    package func addRequest(_ session: String, _ kind: AgentRequest.Kind, _ text: String) -> AgentRequest {
        state.withValue { state in
            defer { state.nextAgentRequest += 1 }
            let request = AgentRequest(
                id: "r\(state.nextAgentRequest)", sessionID: session, kind: kind, text: text,
                created: start.addingTimeInterval(TimeInterval(state.nextAgentRequest)))
            state.requests.append(request)
            return request
        }
    }

    /// Puts `session` on `host`, or this Mac if nil, as one a backend ran
    /// outside Humata.
    package func addImportable(_ session: ImportableSession, on host: String? = nil) {
        state.withValue { $0.importable[host ?? "", default: []].append(session) }
    }

    /// Puts `session` on `host`, or this Mac if nil, as one whose files
    /// couldn't be read.
    package func addSkipped(_ session: SkippedSession, on host: String? = nil) {
        state.withValue { $0.skipped[host ?? "", default: []].append(session) }
    }

    /// Has `host` lose its sessions, as a machine wiped or replaced does; the
    /// Mac keeps its copies of them.
    package func lose(host: String) {
        state.withValue { state in
            for session in state.sessions where session.host == host {
                state.lost.insert(session.id)
            }
        }
    }

    /// Has the session's host lose it; the Mac keeps its copy.
    package func lose(session id: String) {
        state.withValue { _ = $0.lost.insert(id) }
    }

    /// Every session on a host has a copy as of its last turn.
    private func recoveries() -> [Recovery] {
        state.withValue { state in
            state.sessions.compactMap { session in
                guard let host = session.host else { return nil }
                return Recovery(
                    id: session.id, name: session.name, host: host, backedUpTurns: session.turns,
                    turns: session.turns, backedUp: start, onHost: !state.lost.contains(session.id))
            }
            .sorted { ($0.host, $0.name, $0.id) < ($1.host, $1.name, $1.id) }
        }
    }

    private func restoreSession(_ id: String, onto host: String?) throws -> Session {
        try state.withValue { state in
            guard let index = state.sessions.firstIndex(where: { $0.id == id }) else {
                throw DaemonError.refused(
                    .invalidRequest,
                    message: "There is no recovery copy of session \(id); one is kept after each turn on a host.")
            }
            let session = state.sessions[index]
            // One that answers where it is moves only between turns, and not
            // onto where it is already.
            if !state.lost.contains(id) {
                if session.state == .working || session.state == .waitingForApproval {
                    throw DaemonError.refused(
                        .busy, message: "\(session.name) is in a turn; stop it or wait for it, then move it.")
                }
                if session.host == host {
                    throw DaemonError.refused(.invalidRequest, message: "\(session.name) is there already.")
                }
            }
            if let host, state.offline.contains(host) {
                throw DaemonError.refused(
                    .hostUnavailable, message: "Couldn't reach \(host). Check that it is up, then restore again.")
            }
            state.lost.remove(id)
            state.sessions[index].host = host
            return state.sessions[index]
        }
    }

    private func restoreHost(_ host: String) -> HostRestore {
        let lost = recoveries().filter { $0.host == host && !$0.onHost }
        var restored: [Session] = []
        var failed: [RestoreFailure] = []
        for recovery in lost {
            do {
                restored.append(try restoreSession(recovery.id, onto: host))
            } catch {
                failed.append(RestoreFailure(id: recovery.id, message: error.localizedDescription))
            }
        }
        return HostRestore(restored: restored, failed: failed)
    }

    /// Sets what `backend` offers; nil makes listing them fail, as on a
    /// Copilot account without the feature.
    package func setModels(_ models: [ModelChoice]?, for backend: Backend) {
        state.withValue { $0.models[backend] = models }
    }

    /// What the agent has written in the session's copy of the code, by path.
    package func files(of session: String) -> [String: String] {
        state.withValue { state in
            let writes = state.copyOf[session].flatMap { state.copies[$0] } ?? []
            return Self.contents(after: writes)
        }
    }

    /// What the agent has written in the session's folder, by path from it.
    private func folder(of session: String) throws -> [String: String] {
        try known(session)
        guard let directory = state.value.sessions.first(where: { $0.id == session })?.workingDirectory else {
            return [:]
        }
        let prefix = directory.hasSuffix("/") ? directory : directory + "/"
        return files(of: session).reduce(into: [:]) { folder, file in
            // A path not from the root is from the session's folder.
            if !file.key.hasPrefix("/") {
                folder[file.key] = file.value
            } else if file.key.hasPrefix(prefix) {
                folder[String(file.key.dropFirst(prefix.count))] = file.value
            }
        }
    }

    /// Puts a file at `path`, absolute, on `host` or this Mac.
    package func addFile(_ path: String, _ data: Data, on host: String? = nil) {
        state.withValue { $0.hostFiles[host ?? "", default: [:]][path] = data }
    }

    /// Where `place` browses from and the files under it, by path from it.
    private func browsed(_ place: HostPlace) throws -> (root: String, files: [String: Data]) {
        if let session = place.sessionID {
            let root = state.value.sessions.first(where: { $0.id == session })?.workingDirectory ?? ""
            return (root, try folder(of: session).mapValues { Data($0.utf8) })
        }
        let host = place.host ?? ""
        if !host.isEmpty, state.value.offline.contains(host) {
            throw DaemonError.refused(
                .hostUnavailable, message: "Couldn't reach \(host). Check that it is up, then try again.")
        }
        let root = place.folder ?? (host.isEmpty ? "/Users/me" : "/home/me")
        let prefix = root.hasSuffix("/") ? root : root + "/"
        let files = (state.value.hostFiles[host] ?? [:]).reduce(into: [String: Data]()) { files, file in
            if file.key.hasPrefix(prefix) { files[String(file.key.dropFirst(prefix.count))] = file.value }
        }
        return (root, files)
    }

    private func listFolder(_ place: HostPlace, _ directory: String) throws -> Folder {
        let (root, files) = try browsed(place)
        let prefix = directory.isEmpty ? "" : directory + "/"
        var entries: [String: FolderEntry] = [:]
        for (path, data) in files where path.hasPrefix(prefix) {
            let rest = path.dropFirst(prefix.count).split(separator: "/", omittingEmptySubsequences: true)
            guard let name = rest.first.map(String.init), !name.hasPrefix(".") else { continue }
            if rest.count > 1 {
                entries[name] = FolderEntry(name: name, isDirectory: true)
            } else {
                entries[name] = FolderEntry(name: name, isDirectory: false, size: UInt64(data.count))
            }
        }
        let sorted = entries.values.sorted {
            $0.isDirectory != $1.isDirectory
                ? $0.isDirectory : $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
        }
        return Folder(root: root, directory: directory, entries: sorted)
    }

    private func readFile(at place: HostPlace, _ path: String) throws -> FileContents {
        guard let data = try browsed(place).files[path] else {
            throw DaemonError.refused(
                .invalidRequest, message: "\(path) isn't a file in the folder being browsed.")
        }
        let images: Set<String> = ["png", "jpg", "jpeg", "gif", "webp", "heic", "tif", "tiff", "bmp"]
        let isImage = images.contains((path as NSString).pathExtension.lowercased())
        return FileContents(
            path: path, text: isImage ? nil : String(bytes: data, encoding: .utf8), data: isImage ? data : nil,
            size: UInt64(data.count))
    }

    package func history(_ session: String) -> [SessionEvent] {
        state.value.histories[session] ?? []
    }

    package var client: DaemonClient {
        DaemonClient(
            projects: { self.state.value.projects },
            createProject: { name, repositories, instructions in
                try self.createProject(name: name, repositories: repositories, instructions: instructions)
            },
            deleteProject: { project in try self.deleteProject(project) },
            updateProject: { project, update in try self.updateProject(project, update) },
            sessions: { project in
                self.state.withValue { state in
                    state.sessions.filter {
                        (project == nil || $0.projectID == project) && !state.lost.contains($0.id)
                    }.map { session in
                        var session = session
                        session.hostOffline = session.host.map(state.offline.contains) ?? false
                        session.asking = state.requests.contains { $0.sessionID == session.id }
                        session.itemTitle = session.itemID.flatMap { item in
                            state.items.first { $0.id == item }?.title
                        }
                        return session
                    }
                }
            },
            start: { request in try self.startSession(request) },
            importable: { host in
                try self.reachable(host)
                return self.state.value.importable[host ?? "", default: []].sorted { $0.updated > $1.updated }
            },
            importSession: { session, host, projectID in
                try self.importSession(session, on: host, projectID: projectID)
            },
            models: { host, backend, _ in
                try self.reachable(host)
                guard let models = self.state.value.models[backend] else {
                    throw DaemonError.refused(
                        .backendFailed, message: "Couldn't list the models. Type the model's name instead.")
                }
                return models
            },
            skipped: { host in
                try self.reachable(host)
                return self.state.value.skipped[host ?? "", default: []]
            },
            removeSkipped: { session, host in
                try self.reachable(host)
                try self.state.withValue { state in
                    guard state.skipped[host ?? "", default: []].contains(session) else {
                        throw DaemonError.refused(
                            .unknownSession,
                            message: "\(session.id) isn't a skipped session; it may have been removed already.")
                    }
                    state.skipped[host ?? ""]?.removeAll { $0 == session }
                }
            },
            send: { session, text, mode in
                try self.send(session, text, mode)
                // Writing to it answers what it asked.
                self.answerRequests { $0.sessionID == session && $0.kind == .question }
            },
            sendImages: { session, text, images, mode in
                try self.send(session, text, mode, images: images)
                self.answerRequests { $0.sessionID == session && $0.kind == .question }
            },
            readImage: { session, name in
                try self.known(session)
                guard let image = self.state.value.images[session]?[name] else {
                    throw DaemonError.refused(.unknownImage, message: "Session \(session) has no image \(name).")
                }
                return image
            },
            approve: { session, request, approved in try self.answer(session, request, approved: approved) },
            setApprovalMode: { session, mode in try self.setApprovalMode(session, mode) },
            abort: { session in try self.abort(session) },
            stopAll: { session in
                try self.dropQueue(session)
                try self.abort(session)
            },
            rewind: { session, before, files in try self.rewind(session, before: before, files: files) },
            undoRewind: { session in try self.undoRewind(session) },
            unqueue: { session, queueID in try self.unqueue(session, queueID) },
            editQueued: { session, queueID, text in try self.editQueued(session, queueID, text) },
            fork: { session, before, name, chatOnly in
                try self.fork(session, before: before, name: name, chatOnly: chatOnly)
            },
            startWork: { session, item in try self.startWork(session, on: item) },
            proposals: { session in try self.proposals(session) },
            outsideProposals: { project in self.outsideProposals(project) },
            applyProposal: { session, proposal, title, spec in
                try self.applyProposal(session, proposal, title: title, spec: spec)
            },
            declineProposal: { session, proposal in try self.declineProposal(session, proposal) },
            update: { session, update in try self.update(session, update) },
            delete: { session in try self.delete(session) },
            markSeen: { session, turns in try self.markSeen(session, turns) },
            changes: { session, query in try self.changes(session, query) },
            editChanges: { session, edit in try self.editChanges(session, edit) },
            markViewed: { session, file, blob, viewed in try self.markViewed(session, file, blob, viewed) },
            createThread: { session, thread in try self.createThread(session, thread) },
            replyToThread: { thread, text in try self.replyToThread(thread, text) },
            resolveThread: { thread, resolved in try self.resolveThread(thread, resolved) },
            deleteComment: { comment in try self.deleteComment(comment) },
            threads: { session in try self.threads(session) },
            sendReview: { session in try self.sendReview(session) },
            files: { session in
                let contents = try self.folder(of: session)
                return ChatFiles(
                    files: contents.keys.sorted().map { path in
                        ChatFile(path: path, size: UInt64(contents[path]?.utf8.count ?? 0))
                    })
            },
            readFile: { session, path in
                guard let text = try self.folder(of: session)[path] else {
                    throw DaemonError.refused(
                        .invalidRequest, message: "\(path) isn't a file in the session's folder.")
                }
                return FileContents(path: path, text: text)
            },
            fileVersions: { session, path in try self.fileVersions(session, path) },
            readFileVersion: { session, path, turn in try self.readFileVersion(session, path, turn) },
            restoreFileVersion: { session, path, turn in try self.restoreFileVersion(session, path, turn) },
            listFolder: { place, directory in try self.listFolder(place, directory) },
            readFileAt: { place, path in try self.readFile(at: place, path) },
            items: { project in try self.items(project) },
            createItem: { project, fields in try self.createItem(project, fields) },
            updateItem: { item, update in
                let updated = try self.updateItem(item, update)
                if update.status != nil {
                    // Moved by hand, which answers the moves agents asked for.
                    self.answerRequests {
                        if case .move(item, _) = $0.kind { true } else { false }
                    }
                }
                return updated
            },
            deleteItem: { item in try self.deleteItem(item) },
            boardLog: { project, from, limit in
                self.state.withValue { state in
                    let all = state.items + state.removed.values
                    let changes = state.changes.reversed().filter { change in
                        change.id >= (from ?? 0) && all.first { $0.id == change.itemID }?.projectID == project
                    }
                    return limit.map { Array(changes.prefix(max($0, 0))) } ?? changes
                }
            },
            itemLog: { project, item in
                self.state.withValue { state in
                    let all = state.items + state.removed.values
                    guard all.first(where: { $0.id == item })?.projectID == project else { return [] }
                    return state.changes.reversed().filter { $0.itemID == item }
                }
            },
            revert: { change in try self.revert(change) },
            inbox: { Inbox(entries: self.inbox(), approvals: self.pendingApprovals()) },
            approvals: { self.approvals() },
            decideApprovals: { ids, approve, scopes, seconds in
                try self.decideApprovals(ids, approve: approve, scopes: scopes, seconds: seconds)
            },
            revokeGrant: { grant in try self.revokeGrant(grant) },
            storage: { self.state.value.storage },
            setStorageSettings: { autoDelete, days in
                self.state.withValue { state in
                    state.storage.autoDelete = autoDelete
                    state.storage.days = days
                    return state.storage
                }
            },
            cleanUpStorage: { cleanup in self.cleanUp(cleanup) },
            skills: { self.state.value.skills },
            pickSkill: { name, candidates, winner in try self.pickSkill(name, candidates, winner) },
            plugins: { self.state.value.plugins },
            setProjectPlugin: { project, agent, id, enabled in
                try self.setProjectPlugin(project, agent, id, enabled)
            },
            sessionDefaults: { self.state.value.sessionDefaults },
            setSessionDefaults: { project, defaults in try self.setSessionDefaults(project, defaults) },
            answerRequest: { request, answer in try self.answerRequest(request, answer) },
            history: { session in
                try self.known(session)
                return self.history(session)
            },
            hosts: { self.state.value.hosts },
            coordinator: { RemoteHost.coordinator(envNames: (self.state.value.env[""] ?? [:]).keys.sorted()) },
            addHost: { name, destination in try self.addHost(name, destination) },
            updateHost: { host, update in try self.updateHost(host, update) },
            removeHost: { host in try self.removeHost(host) },
            checkHost: { host in try self.checkHost(host) },
            installHost: { host in try self.installHost(host) },
            linkCommands: { check in self.linkCommands(check) },
            setModelServer: { _ in },
            sshHosts: { self.sshConfigHosts },
            recoveries: { self.recoveries() },
            restoreSession: { session, host in try self.restoreSession(session, onto: host) },
            restoreHost: { host in self.restoreHost(host) },
            openTerminal: { place, _ in try self.openTerminal(place) },
            terminals: { host in self.terminals(on: host) },
            typeInTerminal: { terminal, data in try self.type(data, in: terminal.id) },
            resizeTerminal: { terminal, _ in try self.terminal(terminal.id) },
            closeTerminal: { terminal in try self.closeTerminal(terminal.id) },
            attachTerminal: { terminal in self.attachTerminal(terminal.id) },
            forwardPort: { host, port, session in try self.forwardPort(host, port, session) },
            forwards: { self.state.value.forwards },
            stopForward: { id in try self.stopForward(id) },
            search: { query in try self.search(query) },
            land: { session, request in try self.land(session, request) },
            pullRequests: { of, _ in self.pullRequests(of) },
            pullRefresh: { self.state.value.pullRefresh },
            setPullRefresh: { refresh in try self.setPullRefresh(refresh) },
            updateFromBase: { session in try self.updateFromBase(session) },
            importIssues: { project in try self.importIssues(project) },
            watch: { session, after in self.watch(session, after: after) }
        )
    }

    static func unknown(_ session: String) -> DaemonError {
        .refused(.unknownSession, message: "There is no session \(session); list sessions to find its id.")
    }

    func known(_ session: String) throws {
        guard state.value.histories[session] != nil else { throw Self.unknown(session) }
    }

    private func deleteProject(_ project: String) throws {
        try state.withValue { state in
            guard let index = state.projects.firstIndex(where: { $0.id == project }) else {
                throw DaemonError.refused(.unknownProject, message: "There is no project \(project).")
            }
            guard !state.sessions.contains(where: { $0.projectID == project && !$0.archived }) else {
                throw DaemonError.refused(
                    .busy, message: "The project still has sessions working in it; archive them first.")
            }
            state.projects.remove(at: index)
            for session in state.sessions.indices where state.sessions[session].projectID == project {
                state.sessions[session].projectID = nil
                state.sessions[session].itemID = nil
            }
        }
    }

    private func createProject(name: String, repositories: [Repository], instructions: String) throws -> Project {
        let repositories = try Self.named(repositories)
        return state.withValue { state in
            let project = Project(
                id: "p\(state.nextProject)", name: name, repositories: repositories, instructions: instructions,
                created: start + Double(state.nextProject))
            state.nextProject += 1
            state.projects.append(project)
            return project
        }
    }

    /// Refuses a host that isn't known, or can't be reached.
    private func reachable(_ host: String?) throws {
        guard let host else { return }
        let state = state.value
        guard state.hosts.contains(where: { $0.name == host }) else { throw Self.unknownHost(host) }
        guard !state.offline.contains(host), state.installed.contains(host) else {
            throw DaemonError.refused(
                .hostUnavailable, message: "Couldn't reach \(host). Check it in Hosts, then try again.")
        }
    }

    private func importSession(_ found: ImportableSession, on host: String?, projectID: String?) throws -> Session {
        try reachable(host)
        let session = try state.withValue { state in
            guard state.importable[host ?? "", default: []].contains(found) else {
                if let session = state.imported[found.id] {
                    throw DaemonError.refused(
                        .alreadyImported, message: "\(found.id) was already imported, as \(session); open that one.")
                }
                throw DaemonError.refused(.unknownImport, message: "There is no session \(found.id) to import.")
            }
            if let projectID, !state.projects.contains(where: { $0.id == projectID }) {
                throw DaemonError.refused(.unknownProject, message: "There is no project \(projectID).")
            }
            let id = "s\(state.nextSession)"
            state.nextSession += 1
            state.importable[host ?? ""]?.removeAll { $0 == found }
            state.imported[found.id] = id
            let session = Session(
                id: id, name: found.summary, backend: found.backend, workingDirectory: found.workingDirectory,
                host: host, projectID: projectID)
            state.sessions.append(session)
            state.histories[id] = []
            state.copies[id] = []
            state.copyOf[id] = id
            return session
        }
        _ = append(.userMessage(found.summary), to: session.id)
        _ = append(.turnEnded(aborted: false), to: session.id)
        _ = append(.idle, to: session.id)
        // What happened outside Humata isn't news: it was read there.
        return state.withValue { state in
            guard let index = state.sessions.firstIndex(where: { $0.id == session.id }) else { return session }
            state.sessions[index].seenTurns = state.sessions[index].turns
            return state.sessions[index]
        }
    }

    private func startSession(_ request: StartRequest) throws -> Session {
        try reachable(request.host)
        let unpicked = state.value.skills.clashes.filter { $0.winner == nil }.map(\.name)
        if !unpicked.isEmpty {
            throw DaemonError.refused(
                .skillClash,
                message:
                    "This session would get more than one skill named \(unpicked.joined(separator: ", ")); "
                    + "pick which wins in Settings › Skills, then start it again."
            )
        }
        return try state.withValue { state in
            let id = "s\(state.nextSession)"
            var directory = request.workingDirectory
            var projectID = request.projectID
            if let itemID = request.itemID {
                guard let item = state.items.first(where: { $0.id == itemID }) else { throw Self.unknownItem(itemID) }
                guard projectID == nil || projectID == item.projectID else {
                    throw DaemonError.refused(
                        .invalidRequest, message: "\(item.title) is in another project; start the session in its own.")
                }
                projectID = item.projectID
            }
            if let projectID {
                guard let project = state.projects.first(where: { $0.id == projectID }) else {
                    throw DaemonError.refused(.unknownProject, message: "There is no project \(projectID).")
                }
                directory = directory ?? (request.planning ? "/plans/\(id)" : Self.copy(of: project, for: id))
            }
            // A chat in no folder gets one of its own.
            let folder = directory ?? "/chats/\(id)"
            let session = Session(
                id: id, name: request.name, backend: request.backend,
                workingDirectory: folder, model: request.model, host: request.host,
                approvalMode: request.approvalMode, projectID: projectID, itemID: request.itemID,
                planning: request.planning, effort: request.effort, experimental: request.experimental)
            state.nextSession += 1
            state.sessions.append(session)
            state.histories[session.id] = []
            state.copies[session.id] = []
            state.copyOf[session.id] = session.id
            return session
        }
    }

    /// Where a session's copy of the project is: the repository's folder in
    /// it if there is one, otherwise the folder holding each.
    /// `repositories` as a project keeps them: each named uniquely, and one
    /// added by its URL given a clone's folder.
    static func named(_ repositories: [Repository]) throws -> [Repository] {
        guard !repositories.isEmpty else {
            throw DaemonError.refused(.invalidRequest, message: "A project needs a repository to work in.")
        }
        var named: [Repository] = []
        for repository in repositories {
            let url = repository.url.trimmingCharacters(in: .whitespaces)
            let fromURL = url.split(whereSeparator: { $0 == "/" || $0 == ":" }).last.map {
                String($0).replacingOccurrences(of: ".git", with: "")
            }
            guard url.isEmpty ? repository.path.hasPrefix("/") : Repository.isURL(url) else {
                throw DaemonError.refused(
                    .invalidRequest,
                    message: "The repository \(url.isEmpty ? repository.path : url) must be a URL to clone or "
                        + "an absolute path to a folder on this Mac.")
            }
            let folder = url.isEmpty ? (repository.path as NSString).lastPathComponent : fromURL ?? "repository"
            var name = repository.name.isEmpty ? folder : repository.name
            let base = name
            var suffix = 2
            while named.contains(where: { $0.name == name }) {
                name = "\(base)-\(suffix)"
                suffix += 1
            }
            let path = url.isEmpty ? repository.path : "/Users/me/Library/Caches/Humata/repositories/\(folder)"
            named.append(Repository(name: name, path: path, url: url))
        }
        return named
    }

    static func copy(of project: Project, for session: String) -> String {
        let folder = "/copies/\(session)"
        guard project.repositories.count == 1, let repository = project.repositories.first else { return folder }
        return "\(folder)/\(repository.name)"
    }

    func send(_ session: String, _ text: String, _ mode: SendMode, images: [ImageAttachment] = []) throws {
        try known(session)
        if state.value.sessions.first(where: { $0.id == session })?.archived == true {
            throw DaemonError.refused(
                .archived, message: "Session \(session) is archived; unarchive it to work in it again.")
        }
        let (busy, queued, names) = state.withValue { state -> (Bool, Queued?, [String]) in
            let names = images.map { image in
                let name =
                    "i\(state.images[session, default: [:]].count + 1)."
                    + (image.mediaType.split(separator: "/").last.map(String.init) ?? "png")
                state.images[session, default: [:]][name] = image
                return name
            }
            guard state.paused[session] != nil else { return (false, nil, names) }
            // A running turn can't take images, so a steer with them goes next.
            if mode == .steer, !names.isEmpty {
                defer { state.nextQueued += 1 }
                let queued = Queued(id: "q\(state.nextQueued)", text: text, images: names)
                state.queues[session, default: []].insert(queued, at: 0)
                return (true, queued, names)
            }
            guard mode == .enqueue else { return (true, nil, names) }
            // What's sent while a message waits joins it.
            if let last = state.queues[session]?.indices.last, var joined = state.queues[session]?[last] {
                joined.text = [joined.text, text].filter { !$0.isEmpty }.joined(separator: "\n\n")
                joined.images += names
                state.queues[session]?[last] = joined
                return (true, joined, names)
            }
            defer { state.nextQueued += 1 }
            let queued = Queued(id: "q\(state.nextQueued)", text: text, images: names)
            state.queues[session, default: []].append(queued)
            return (true, queued, names)
        }
        if let queued {
            append(.messageQueued(queueID: queued.id, text: queued.text, images: queued.images), to: session)
        } else if !busy {
            play(session, text, images: names)
        } else if mode == .steer {
            // A turn waiting on a person reads the steer when it resumes.
            append(.userMessage(text), to: session)
        }
    }

    /// Takes the next queued message off the session's queue, if any.
    private func nextQueued(_ session: String) -> Queued? {
        let next = state.withValue { state in
            state.queues[session]?.isEmpty == false ? state.queues[session]?.removeFirst() : nil
        }
        guard let next else { return nil }
        append(.dequeued(queueID: next.id), to: session)
        return next
    }

    private func unqueue(_ session: String, _ queueID: String) throws {
        try known(session)
        let removed = state.withValue { state -> Bool in
            guard let index = state.queues[session]?.firstIndex(where: { $0.id == queueID }) else { return false }
            state.queues[session]?.remove(at: index)
            return true
        }
        guard removed else {
            throw DaemonError.refused(
                .unknownQueuedMessage, message: "Session \(session) has no queued message \(queueID).")
        }
        append(.dequeued(queueID: queueID), to: session)
    }

    private func editQueued(_ session: String, _ queueID: String, _ text: String) throws {
        try known(session)
        let edited = state.withValue { state -> Queued? in
            guard let index = state.queues[session]?.firstIndex(where: { $0.id == queueID }) else { return nil }
            state.queues[session]?[index].text = text
            return state.queues[session]?[index]
        }
        guard let edited else {
            throw DaemonError.refused(
                .unknownQueuedMessage,
                message: "Session \(session)'s queued message \(queueID) already started or was taken out.")
        }
        append(.messageQueued(queueID: queueID, text: text, images: edited.images), to: session)
    }

    /// Plays turns for `text`, then for each queued message, until one
    /// waits for permission or the queue is empty.
    private func play(_ session: String, _ text: String, images: [String] = []) {
        var next: Queued? = Queued(id: "", text: text, images: images)
        while let message = next {
            append(.userMessage(message.text, images: message.images), to: session)
            append(.turnStarted, to: session)
            guard proceed(session, steps: script(message.text)) else { return }
            next = self.nextQueued(session)
        }
    }

    /// Plays `steps` in the running turn. False if it stopped to ask.
    private func proceed(_ session: String, steps: [Step]) -> Bool {
        var steps = steps[...]
        while let step = steps.popFirst() {
            switch step {
            case .say(let text):
                append(.assistantDelta(text), to: session)
                append(.assistantMessage(text), to: session)
            case .think(let text):
                append(.reasoning(text), to: session)
            case .propose, .proposeSpecEdit:
                propose(step, in: session)
            case .run(let command, _), .write(let command, _):
                let (tool, kind) = step.isRun ? ("shell", PermissionKind.shell) : ("write_file", .write)
                let summary = step.isRun ? "Run `\(command)`" : "Write \(command)"
                let (callID, request, mode) = state.withValue { state in
                    defer { state.nextRequest += 1 }
                    let mode = state.sessions.first { $0.id == session }?.approvalMode ?? .askEveryTime
                    return ("c\(state.nextRequest)", "r\(state.nextRequest)", mode)
                }
                let key = step.isRun ? "command" : "path"
                let arguments =
                    (try? JSONSerialization.data(withJSONObject: [key: command]))
                    .flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
                append(.toolStarted(callID: callID, tool: tool, argumentsJSON: arguments), to: session)
                append(.permissionRequested(requestID: request, kind: kind, summary: summary), to: session)
                if mode.allows(kind) {
                    append(.permissionCompleted(requestID: request, approved: true, automatic: true), to: session)
                    complete(session, step, callID: callID, approved: true)
                } else {
                    let paused = Paused(request: request, step: step, callID: callID, rest: Array(steps))
                    state.withValue { $0.paused[session] = paused }
                    return false
                }
            }
        }
        append(.turnEnded(aborted: false), to: session)
        append(.idle, to: session)
        return true
    }

    private func complete(_ session: String, _ step: Step, callID: String, approved: Bool) {
        let output: String
        switch step {
        case .run(_, let ran): output = approved ? ran : "The person denied this."
        case .write(let path, _):
            output = approved ? "Wrote \(path)" : "The person denied this."
        case .say, .think, .propose, .proposeSpecEdit: return
        }
        let event = append(.toolCompleted(callID: callID, success: approved, output: output), to: session)
        if approved, case .write(let path, let contents) = step {
            state.withValue { state in
                guard let copy = state.copyOf[session] else { return }
                state.copies[copy, default: []].append(Write(event: event, path: path, contents: contents))
            }
        }
    }

    private func answer(_ session: String, _ request: String, approved: Bool, automatic: Bool = false) throws {
        try known(session)
        let paused = try state.withValue { state -> Paused in
            guard let paused = state.paused[session], paused.request == request else {
                throw DaemonError.refused(
                    .unknownPermissionRequest, message: "Session \(session) isn't waiting on \(request).")
            }
            state.paused[session] = nil
            return paused
        }
        append(.permissionCompleted(requestID: request, approved: approved, automatic: automatic), to: session)
        complete(session, paused.step, callID: paused.callID, approved: approved)
        resume(session, rest: paused.rest)
    }

    /// Carries on a turn after an answer, then runs what was queued.
    private func resume(_ session: String, rest: [Step]) {
        guard proceed(session, steps: rest) else { return }
        let next = nextQueued(session)
        if let next { play(session, next.text, images: next.images) }
    }

    private func setApprovalMode(_ session: String, _ mode: ApprovalMode) throws {
        try known(session)
        let waiting = state.withValue { state -> Paused? in
            if let index = state.sessions.firstIndex(where: { $0.id == session }) {
                state.sessions[index].approvalMode = mode
            }
            return state.paused[session]
        }
        if let waiting, let kind = waiting.step.permission, mode.allows(kind) {
            try answer(session, waiting.request, approved: true, automatic: true)
        }
    }

    private func abort(_ session: String) throws {
        try known(session)
        let stopped = state.withValue { $0.paused.removeValue(forKey: session) != nil }
        guard stopped else { return }
        append(.turnEnded(aborted: true), to: session)
        let next = nextQueued(session)
        if let next {
            play(session, next.text, images: next.images)
        } else {
            append(.idle, to: session)
        }
    }

    private func dropQueue(_ session: String) throws {
        try known(session)
        let dropped = state.withValue { state in state.queues.removeValue(forKey: session) ?? [] }
        for queued in dropped {
            append(.dequeued(queueID: queued.id, dropped: true), to: session)
        }
    }

    private func rewind(_ session: String, before: String, files: Bool) throws {
        try dropQueue(session)
        try abort(session)
        let history = state.value.histories[session] ?? []
        guard let index = history.firstIndex(where: { $0.id == before }) else {
            throw DaemonError.refused(.unknownEvent, message: "Session \(session) has no event \(before).")
        }
        guard case .userMessage(let text, let images) = history[index].kind else {
            throw DaemonError.refused(
                .invalidRequest, message: "Only one of your messages can be rewound to; pick a message you sent.")
        }
        state.withValue { state in
            state.rewinds[session, default: []].append(history)
            state.histories[session] = Array(history[..<index])
        }
        append(.rewound(text: text, images: images, files: files), to: session)
    }

    private func undoRewind(_ session: String) throws {
        try known(session)
        let history = state.value.histories[session] ?? []
        let undoable = history.last {
            switch $0.kind {
            case .rewound, .rewindUndone, .userMessage: true
            default: false
            }
        }
        guard case .rewound = undoable?.kind, let saved = state.value.rewinds[session]?.last else {
            throw DaemonError.refused(
                .invalidRequest, message: "There's no rewind to undo: a message was sent since, or none was made.")
        }
        state.withValue { state in
            state.rewinds[session]?.removeLast()
            state.histories[session] = saved
        }
        append(.rewindUndone, to: session)
    }

    /// Adds an event to the session's history and tells its watchers.
    /// Returns its id.
    @discardableResult
    func append(_ kind: SessionEvent.Kind, to session: String) -> String {
        let (event, watchers) = state.withValue { state in
            let event = SessionEvent(id: "e\(state.nextEvent)", time: start + Double(state.nextEvent), kind: kind)
            state.nextEvent += 1
            state.histories[session, default: []].append(event)
            if let index = state.sessions.firstIndex(where: { $0.id == session }) {
                state.sessions[index].state = Self.state(after: kind, waiting: state.paused[session] != nil)
                state.sessions[index].lastActive = event.time
                if kind.endsTurn { state.sessions[index].turns += 1 }
                switch kind {
                case .assistantMessage(let text), .failed(let text):
                    state.sessions[index].lastMessage = text.count > 300 ? String(text.prefix(300)) + "…" : text
                default: break
                }
                switch kind {
                case .turnStarted, .turnEnded: state.sessions[index].pending = []
                case .permissionRequested(let request, let permission, let summary):
                    state.sessions[index].pending.append(
                        PendingPermission(requestID: request, kind: permission, summary: summary))
                case .permissionCompleted(let request, _, _, _):
                    state.sessions[index].pending.removeAll { $0.requestID == request }
                default: break
                }
            }
            return (event, Array((state.watchers[session] ?? [:]).values))
        }
        for watcher in watchers {
            watcher.yield(.event(event))
        }
        return event.id
    }

    private static func state(after kind: SessionEvent.Kind, waiting: Bool) -> SessionState {
        switch kind {
        case .idle, .turnEnded, .rewound, .rewindUndone: .idle
        case .permissionRequested: .waitingForApproval
        default: waiting ? .waitingForApproval : .working
        }
    }

    func fork(_ session: String, before: String?, name: String?, chatOnly: Bool) throws -> Session {
        try known(session)
        return try state.withValue { state in
            guard let parent = state.sessions.first(where: { $0.id == session }) else {
                throw Self.unknown(session)
            }
            var history = state.histories[session] ?? []
            if let before {
                guard let index = history.firstIndex(where: { $0.id == before }) else {
                    throw DaemonError.refused(.unknownEvent, message: "Session \(session) has no event \(before).")
                }
                history = Array(history[..<index])
            }
            var fork = parent
            fork.id = "s\(state.nextSession)"
            fork.name = name ?? parent.name
            fork.state = .idle
            fork.parentID = parent.id
            fork.forkedBeforeEventID = before
            fork.archived = false
            fork.keeper = false
            fork.turns = 0
            state.nextSession += 1
            if chatOnly {
                state.copyOf[fork.id] = state.copyOf[parent.id]
            } else {
                let kept = Set(history.map(\.id))
                let writes = state.copyOf[parent.id].flatMap { state.copies[$0] } ?? []
                state.copies[fork.id] = writes.filter { kept.contains($0.event) }
                state.copyOf[fork.id] = fork.id
                let parentCopy = "/copies/\(parent.id)"
                if fork.workingDirectory.hasPrefix(parentCopy) {
                    fork.workingDirectory = "/copies/\(fork.id)" + fork.workingDirectory.dropFirst(parentCopy.count)
                }
            }
            state.sessions.append(fork)
            state.histories[fork.id] = history
            fork.turns = history.filter(\.kind.endsTurn).count
            state.sessions[state.sessions.count - 1] = fork
            return fork
        }
    }

    private func markSeen(_ session: String, _ turns: Int) throws {
        try known(session)
        state.withValue { state in
            guard let index = state.sessions.firstIndex(where: { $0.id == session }) else { return }
            state.sessions[index].seenTurns = max(state.sessions[index].seenTurns, turns)
        }
    }

    private func delete(_ session: String) throws {
        try known(session)
        try state.withValue { state in
            guard let deleted = state.sessions.first(where: { $0.id == session }) else {
                throw Self.unknown(session)
            }
            guard deleted.archived else {
                throw DaemonError.refused(
                    .invalidRequest, message: "\(deleted.name) isn't archived; archive it first.")
            }
            if let fork = state.sessions.first(where: { $0.parentID == session }) {
                throw DaemonError.refused(
                    .invalidRequest, message: "\(fork.name) was forked from \(deleted.name); delete it first.")
            }
            guard !deleted.hostOffline else {
                throw DaemonError.refused(
                    .hostUnavailable, message: "\(deleted.name)'s host can't be reached; try again once it's back.")
            }
            state.sessions.removeAll { $0.id == session }
            state.histories[session] = nil
            state.requests.removeAll { $0.sessionID == session }
        }
    }

    private func update(_ session: String, _ update: SessionUpdate) throws -> Session {
        try known(session)
        return try state.withValue { state in
            guard let index = state.sessions.firstIndex(where: { $0.id == session }) else {
                throw Self.unknown(session)
            }
            if update.archived == true, state.paused[session] != nil {
                throw DaemonError.refused(
                    .busy, message: "Session \(session) is working; stop its turn, then archive it.")
            }
            if let name = update.name { state.sessions[index].name = name }
            if let archived = update.archived { state.sessions[index].archived = archived }
            if let keeper = update.keeper { state.sessions[index].keeper = keeper }
            if let experimental = update.experimental { state.sessions[index].experimental = experimental }
            if let effort = update.effort { state.sessions[index].effort = effort.isEmpty ? nil : effort }
            switch update.placement {
            case .item(let itemID):
                guard let item = state.items.first(where: { $0.id == itemID }) else { throw Self.unknownItem(itemID) }
                guard item.projectID == state.sessions[index].projectID else {
                    throw DaemonError.refused(
                        .invalidRequest, message: "\(item.title) is in another project than session \(session).")
                }
                state.sessions[index].itemID = itemID
            case .project: state.sessions[index].itemID = nil
            case nil: break
            }
            return state.sessions[index]
        }
    }

    private func watch(_ session: String, after: String?) -> AsyncThrowingStream<WatchItem, any Error> {
        AsyncThrowingStream { continuation in
            let id = UUID()
            let known = state.withValue { state -> Bool in
                guard let history = state.histories[session] else { return false }
                let unsent = after.flatMap { after in history.lastIndex { $0.id == after } }.map { $0 + 1 } ?? 0
                for event in history[unsent...] {
                    continuation.yield(.event(event))
                }
                continuation.yield(.historyLoaded)
                state.watchers[session, default: [:]][id] = continuation
                return true
            }
            guard known else {
                continuation.finish(throwing: Self.unknown(session))
                return
            }
            continuation.onTermination = { _ in
                self.state.withValue { $0.watchers[session]?[id] = nil }
            }
        }
    }
}

// The board.
extension InMemoryDaemon {
    static func unknownItem(_ item: String) -> DaemonError {
        .refused(.unknownItem, message: "There is no task or subtask \(item); list the board to find its id.")
    }

    /// The project's items as shown, ordered as the daemon orders them.
    private func items(_ project: String) throws -> [Item] {
        try state.withValue { state in
            guard state.projects.contains(where: { $0.id == project }) else {
                throw DaemonError.refused(.unknownProject, message: "There is no project \(project).")
            }
            let items = state.items.filter { $0.projectID == project }.map { item in
                var item = item
                item.sessionIDs = state.sessions.filter { $0.itemID == item.id }.map(\.id)
                return item
            }
            let ordered = { (items: [Item]) in
                items.sorted { ($0.priority, $1.created) > ($1.priority, $0.created) }
            }
            let sessions = state.sessions.map { session in
                var session = session
                session.asking = state.requests.contains { $0.sessionID == session.id }
                return session
            }
            let shown = Self.show(items, sessions: sessions)
            return ordered(shown.filter { $0.parentID == nil }).flatMap { task in
                [task] + ordered(shown.filter { $0.parentID == task.id })
            }
        }
    }

    /// What a session says about its item: waiting or failed is Waiting on
    /// you, working is In progress, a finished turn is In review.
    private static func said(by session: Session) -> ItemStatus? {
        guard !session.archived else { return nil }
        if session.asking { return .waitingOnYou }
        switch session.state {
        case .waitingForApproval, .failed: return .waitingOnYou
        case .working: return .inProgress
        case .idle: return session.turns > 0 ? .inReview : nil
        }
    }

    private static func show(_ items: [Item], sessions: [Session]) -> [Item] {
        let rank: [ItemStatus: Int] = [.waitingOnYou: 3, .inProgress: 2, .inReview: 1]
        let parents = Dictionary(uniqueKeysWithValues: items.map { ($0.id, $0.parentID) })
        var said: [String: [ItemStatus]] = [:]
        for session in sessions {
            guard let item = session.itemID, let status = Self.said(by: session) else { continue }
            said[item, default: []].append(status)
            if let parent = parents[item] ?? nil { said[parent, default: []].append(status) }
        }
        return items.map { item in
            var item = item
            let given = item.fields.status
            item.status =
                given.closed ? given : (said[item.id]?.max { rank[$0, default: 0] < rank[$1, default: 0] } ?? given)
            return item
        }
    }

    func createItem(_ project: String, _ fields: ItemFields, by author: BoardAuthor = .app) throws -> Item {
        try state.withValue { state in
            guard state.projects.contains(where: { $0.id == project }) else {
                throw DaemonError.refused(.unknownProject, message: "There is no project \(project).")
            }
            guard !fields.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw DaemonError.refused(.invalidRequest, message: "An item needs a title.")
            }
            if let parentID = fields.parentID {
                guard let parent = state.items.first(where: { $0.id == parentID }) else {
                    throw Self.unknownItem(parentID)
                }
                guard parent.projectID == project, parent.parentID == nil else {
                    throw DaemonError.refused(
                        .invalidRequest, message: "A subtask goes under a task in the same project.")
                }
            }
            let time = start + Double(1000 + state.nextChange)
            let item = Item(id: "i\(state.nextItem)", projectID: project, fields: fields, created: time)
            state.nextItem += 1
            state.items.append(item)
            Self.log(&state, item: item.id, before: nil, after: fields, time: time, author: author)
            return item
        }
    }

    func updateItem(_ itemID: String, _ update: ItemUpdate, by author: BoardAuthor = .app) throws -> Item {
        let projectID = try state.withValue { state in
            guard let index = state.items.firstIndex(where: { $0.id == itemID }) else {
                throw Self.unknownItem(itemID)
            }
            let before = state.items[index].fields
            var after = before
            if let title = update.title { after.title = title }
            if let spec = update.spec { after.spec = spec }
            if let status = update.status { after.status = status }
            if let priority = update.priority { after.priority = priority }
            if after != before {
                let time = start + Double(1000 + state.nextChange)
                state.items[index].fields = after
                state.items[index].updated = time
                Self.log(&state, item: itemID, before: before, after: after, time: time, author: author)
            }
            return state.items[index].projectID
        }
        return try shown(itemID, in: projectID)
    }

    private func deleteItem(_ itemID: String) throws {
        try state.withValue { state in
            guard let index = state.items.firstIndex(where: { $0.id == itemID }) else {
                throw Self.unknownItem(itemID)
            }
            let item = state.items[index]
            guard !state.items.contains(where: { $0.parentID == itemID }) else {
                throw DaemonError.refused(.invalidRequest, message: "\(item.title) has subtasks; remove them first.")
            }
            guard !state.sessions.contains(where: { $0.itemID == itemID && !$0.archived }) else {
                throw DaemonError.refused(.busy, message: "\(item.title) has sessions; archive them first.")
            }
            for session in state.sessions.indices where state.sessions[session].itemID == itemID {
                state.sessions[session].itemID = nil
            }
            let time = start + Double(1000 + state.nextChange)
            state.items.remove(at: index)
            state.removed[itemID] = item
            Self.log(&state, item: itemID, before: item.fields, after: nil, time: time, author: .app)
        }
    }

    private func shown(_ itemID: String, in project: String) throws -> Item {
        guard let item = try items(project).first(where: { $0.id == itemID }) else { throw Self.unknownItem(itemID) }
        return item
    }

    private static func log(
        _ state: inout State, item: String, before: ItemFields?, after: ItemFields?, time: Date,
        reverts: UInt64? = nil, author: BoardAuthor
    ) {
        state.changes.append(
            BoardChange(
                id: state.nextChange, itemID: item, time: time, before: before, after: after, reverts: reverts,
                author: author))
        state.nextChange += 1
    }

    /// Puts the item back as it was before the change, removing it if the
    /// change added it. A field changed again since is kept.
    private func revert(_ changeID: UInt64) throws -> Reverted {
        var keptFields: [ItemField] = []
        let kept = try state.withValue { state -> (String, String)? in
            guard let change = state.changes.first(where: { $0.id == changeID }) else {
                throw DaemonError.refused(
                    .unknownBoardChange, message: "There is no board change \(changeID); read the board's log.")
            }
            let time = start + Double(1000 + state.nextChange)
            let index = state.items.firstIndex { $0.id == change.itemID }
            guard let before = change.before else {
                guard let index else { return nil }
                let item = state.items[index]
                let inUse =
                    state.items.contains { $0.parentID == item.id } || state.sessions.contains { $0.itemID == item.id }
                guard !inUse else {
                    throw DaemonError.refused(
                        .busy, message: "\(item.title) has subtasks or sessions; move or remove them first.")
                }
                state.items.remove(at: index)
                state.removed[item.id] = item
                Self.log(
                    &state, item: item.id, before: item.fields, after: nil, time: time, reverts: changeID, author: .app)
                return nil
            }
            let projectID: String
            if let index {
                let current = state.items[index].fields
                var back = current
                let after = change.after ?? current
                for field in change.fields {
                    guard !current.differs(from: after, in: field) else {
                        keptFields.append(field)
                        continue
                    }
                    switch field {
                    case .title: back.title = before.title
                    case .spec: back.spec = before.spec
                    case .status: back.status = before.status
                    case .priority: back.priority = before.priority
                    }
                }
                projectID = state.items[index].projectID
                if back != current {
                    state.items[index].fields = back
                    state.items[index].updated = time
                    Self.log(
                        &state, item: change.itemID, before: current, after: back, time: time, reverts: changeID,
                        author: .app)
                }
            } else {
                guard var item = state.removed[change.itemID] else { throw Self.unknownItem(change.itemID) }
                if let parent = before.parentID, !state.items.contains(where: { $0.id == parent }) {
                    throw DaemonError.refused(
                        .invalidRequest, message: "Its task was removed; put the task back first.")
                }
                item.fields = before
                item.updated = time
                projectID = item.projectID
                state.removed[item.id] = nil
                state.items.append(item)
                Self.log(
                    &state, item: change.itemID, before: nil, after: before, time: time, reverts: changeID,
                    author: .app)
            }
            return (change.itemID, projectID)
        }
        guard let (itemID, projectID) = kept else { return Reverted(item: nil) }
        return Reverted(item: try shown(itemID, in: projectID), kept: keptFields)
    }

    private static func unknownHost(_ host: String) -> DaemonError {
        .refused(.unknownHost, message: "There is no host \(host); add it in Hosts first.")
    }

    private func addHost(_ name: String, _ destination: String) throws -> RemoteHost {
        let name = name.trimmingCharacters(in: .whitespaces)
        let destination = destination.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty, name.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "-" || $0 == "_" || $0 == "." })
        else {
            throw DaemonError.refused(
                .invalidRequest, message: "A host's name may hold only letters, digits, '-', '_' and '.'.")
        }
        guard !destination.isEmpty else {
            throw DaemonError.refused(.invalidRequest, message: "Give the host's SSH destination, such as user@box.")
        }
        return try state.withValue { state in
            guard !state.hosts.contains(where: { $0.name == name }) else {
                throw DaemonError.refused(
                    .invalidRequest, message: "There is a host \(name) already; pick another name.")
            }
            let host = RemoteHost(name: name, sshDestination: destination)
            state.hosts.append(host)
            return host
        }
    }

    private func updateHost(_ name: String, _ update: HostUpdate) throws -> RemoteHost {
        try state.withValue { state in
            if name.isEmpty {
                guard update.setupScript == nil else {
                    throw DaemonError.refused(
                        .invalidRequest, message: "This Mac has no setup script; install what its agents need yourself."
                    )
                }
                var env = state.env[""] ?? [:]
                for unset in update.unsetEnv { env[unset] = nil }
                env.merge(update.setEnv) { _, new in new }
                state.env[""] = env
                return RemoteHost.coordinator(envNames: env.keys.sorted())
            }
            guard let index = state.hosts.firstIndex(where: { $0.name == name }) else { throw Self.unknownHost(name) }
            var env = state.env[name] ?? [:]
            for unset in update.unsetEnv { env[unset] = nil }
            env.merge(update.setEnv) { _, new in new }
            state.env[name] = env
            state.hosts[index].envNames = env.keys.sorted()
            if let script = update.setupScript { state.hosts[index].setupScript = script }
            return state.hosts[index]
        }
    }

    private func removeHost(_ name: String) throws {
        try state.withValue { state in
            guard state.hosts.contains(where: { $0.name == name }) else { throw Self.unknownHost(name) }
            state.hosts.removeAll { $0.name == name }
            state.env[name] = nil
            state.installed.remove(name)
            state.sessions.removeAll { $0.host == name }
        }
    }

    private func checkHost(_ name: String) throws -> HostHealth {
        try state.withValue { state in
            guard let host = state.hosts.first(where: { $0.name == name }) else { throw Self.unknownHost(name) }
            if state.offline.contains(name) {
                return .unreachable("ssh couldn't connect to \(host.sshDestination ?? name). Check that it is up.")
            }
            guard state.installed.contains(name) else {
                return .unreachable("Humata's program isn't on \(name) yet. Install it, then check again.")
            }
            let env = state.env[name] ?? [:]
            return .reachable(
                HostStatus(
                    programVersion: "0.0.0", platform: "Linux x86_64", freeDiskBytes: 120_000_000_000,
                    copilot: .success("1.0.0"),
                    githubToken: ["GH_TOKEN", "GITHUB_TOKEN", "COPILOT_GITHUB_TOKEN"].contains { env[$0] != nil },
                    setup: host.setupScript.isEmpty ? nil : SetupResult(succeeded: true, output: "", time: start),
                    claude: .success("2.1.322 (Claude Code)"),
                    anthropicKey: ["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"].contains { env[$0] != nil }))
        }
    }

    private func installHost(_ name: String) throws -> String {
        try state.withValue { state in
            guard state.hosts.contains(where: { $0.name == name }) else { throw Self.unknownHost(name) }
            guard !state.offline.contains(name) else {
                throw DaemonError.refused(
                    .hostUnavailable, message: "ssh couldn't connect to \(name). Check that it is up, then retry.")
            }
            state.installed.insert(name)
            return "0.0.0"
        }
    }

    private func linkCommands(_ check: Bool) -> CommandLinks {
        state.withValue { state in
            if !check {
                for index in state.commandLinks.links.indices
                where [.missing, .stale].contains(state.commandLinks.links[index].state) {
                    state.commandLinks.links[index].state = .linked
                }
            }
            return state.commandLinks
        }
    }

    private func answerRequest(_ id: String, _ answer: RequestAnswer) throws {
        guard let request = state.value.requests.first(where: { $0.id == id }) else {
            throw DaemonError.refused(.unknownRequest, message: "Request \(id) was answered already.")
        }
        switch (answer, request.kind) {
        case (.reply(let text), _):
            try send(request.sessionID, text, .enqueue)
        case (.approve, .move(let item, let status)), (.move(let status), .move(let item, _)):
            _ = try updateItem(item, ItemUpdate(status: status), by: .session(request.sessionID))
        case (.approve, .question), (.move, .question):
            throw DaemonError.refused(.invalidRequest, message: "A question is answered with a reply.")
        case (.dismiss, _): break
        }
        state.withValue { $0.requests.removeAll { $0.id == id } }
    }

    private func answerRequests(where answered: (AgentRequest) -> Bool) {
        state.withValue { $0.requests.removeAll(where: answered) }
    }

    /// Sessions waiting on you: approvals, then questions and moves, then
    /// failures, then replies.
    private func inbox() -> [InboxEntry] {
        let state = state.value
        let project = { (session: Session) in state.projects.first { $0.id == session.projectID }?.name }
        let asked = state.requests.compactMap { request -> InboxEntry? in
            guard let session = state.sessions.first(where: { $0.id == request.sessionID }), !session.archived
            else { return nil }
            let reason: InboxReason = if case .question = request.kind { .question } else { .move }
            return InboxEntry(
                session: session, reason: reason, projectName: project(session),
                itemTitle: state.items.first { $0.id == session.itemID }?.title, request: request)
        }
        let entries =
            state.sessions.compactMap { session -> InboxEntry? in
                guard !session.archived else { return nil }
                let item = state.items.first { $0.id == session.itemID }
                let reason: InboxReason
                switch session.state {
                case .waitingForApproval: reason = .approval
                case .failed: reason = .failed
                case .idle
                where session.turns > session.seenTurns && item?.fields.status.closed != true
                    && !state.requests.contains(where: { $0.sessionID == session.id }):
                    reason = .reply
                case .idle, .working: return nil
                }
                return InboxEntry(
                    session: session, reason: reason, projectName: project(session), itemTitle: item?.title)
            } + asked
        // Within a rank, the one waiting longest comes first.
        let order: [[InboxReason]] = [[.approval], [.question, .move], [.failed], [.reply]]
        return order.flatMap { reasons in
            entries.filter { reasons.contains($0.reason) }
                .sorted { ($0.session.lastActive ?? .distantPast) < ($1.session.lastActive ?? .distantPast) }
        }
    }
}

extension SessionEvent.Kind {
    fileprivate var endsTurn: Bool {
        if case .turnEnded = self { true } else { false }
    }
}

extension InMemoryDaemon.Step {
    fileprivate var isRun: Bool {
        if case .run = self { true } else { false }
    }

    /// The permission it needs; nil if it needs none.
    fileprivate var permission: PermissionKind? {
        switch self {
        case .run: .shell
        case .write: .write
        case .say, .think, .propose, .proposeSpecEdit: nil
        }
    }
}
