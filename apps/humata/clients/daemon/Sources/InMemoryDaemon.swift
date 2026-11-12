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

    struct State {
        var projects: [Project] = []
        var sessions: [Session] = []
        var histories: [String: [SessionEvent]] = [:]
        var paused: [String: Paused] = [:]
        var queues: [String: [String]] = [:]
        /// Each copy of the code's writes, oldest first, by the id of the
        /// session that made the copy.
        var copies: [String: [Write]] = [:]
        /// The copy each session works in, by session id.
        var copyOf: [String: String] = [:]
        var watchers: [String: [UUID: AsyncThrowingStream<SessionEvent, any Error>.Continuation]] = [:]
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
        var reviews: [Review] = []
        var nextThread = 1
        var nextComment = 1
    }

    let state = LockIsolated(State())
    private let script: @Sendable (String) -> [Step]
    let start: Date
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

    package func history(_ session: String) -> [SessionEvent] {
        state.value.histories[session] ?? []
    }

    package var client: DaemonClient {
        DaemonClient(
            projects: { self.state.value.projects },
            createProject: { name, repositories, instructions in
                try self.createProject(name: name, repositories: repositories, instructions: instructions)
            },
            sessions: { project in
                self.state.withValue { state in
                    state.sessions.filter { project == nil || $0.projectID == project }.map { session in
                        var session = session
                        session.hostOffline = session.host.map(state.offline.contains) ?? false
                        return session
                    }
                }
            },
            start: { request in try self.startSession(request) },
            send: { session, text, mode in try self.send(session, text, mode) },
            approve: { session, request, approved in try self.answer(session, request, approved: approved) },
            setApprovalMode: { session, mode in try self.setApprovalMode(session, mode) },
            abort: { session in try self.abort(session) },
            fork: { session, before, name, chatOnly in
                try self.fork(session, before: before, name: name, chatOnly: chatOnly)
            },
            update: { session, update in try self.update(session, update) },
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
            items: { project in try self.items(project) },
            createItem: { project, fields in try self.createItem(project, fields) },
            updateItem: { item, update in try self.updateItem(item, update) },
            boardLog: { project in
                self.state.withValue { state in
                    let all = state.items + state.removed.values
                    return state.changes.reversed().filter { change in
                        all.first { $0.id == change.itemID }?.projectID == project
                    }
                }
            },
            revert: { change in try self.revert(change) },
            inbox: { self.inbox() },
            history: { session in
                try self.known(session)
                return self.history(session)
            },
            hosts: { self.state.value.hosts },
            addHost: { name, destination in try self.addHost(name, destination) },
            updateHost: { host, update in try self.updateHost(host, update) },
            removeHost: { host in try self.removeHost(host) },
            checkHost: { host in try self.checkHost(host) },
            installHost: { host in try self.installHost(host) },
            sshHosts: { self.sshConfigHosts },
            watch: { session in self.watch(session) }
        )
    }

    static func unknown(_ session: String) -> DaemonError {
        .refused(.unknownSession, message: "There is no session \(session); list sessions to find its id.")
    }

    func known(_ session: String) throws {
        guard state.value.histories[session] != nil else { throw Self.unknown(session) }
    }

    private func createProject(name: String, repositories: [Repository], instructions: String) throws -> Project {
        guard !repositories.isEmpty else {
            throw DaemonError.refused(.invalidRequest, message: "A project needs a repository to work in.")
        }
        var named: [Repository] = []
        for repository in repositories {
            guard repository.path.hasPrefix("/") else {
                throw DaemonError.refused(
                    .invalidRequest,
                    message: "The repository \(repository.path) must be an absolute path to a folder on this Mac.")
            }
            var name = repository.name.isEmpty ? (repository.path as NSString).lastPathComponent : repository.name
            let base = name
            var suffix = 2
            while named.contains(where: { $0.name == name }) {
                name = "\(base)-\(suffix)"
                suffix += 1
            }
            named.append(Repository(name: name, path: repository.path))
        }
        let repositories = named
        return state.withValue { state in
            let project = Project(
                id: "p\(state.nextProject)", name: name, repositories: repositories, instructions: instructions,
                created: start + Double(state.nextProject))
            state.nextProject += 1
            state.projects.append(project)
            return project
        }
    }

    private func startSession(_ request: StartRequest) throws -> Session {
        try state.withValue { state in
            if let host = request.host {
                guard state.hosts.contains(where: { $0.name == host }) else { throw Self.unknownHost(host) }
                guard !state.offline.contains(host), state.installed.contains(host) else {
                    throw DaemonError.refused(
                        .hostUnavailable, message: "Couldn't reach \(host). Check it in Hosts, then try again.")
                }
            }
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
                directory = directory ?? Self.copy(of: project, for: id)
            }
            // A chat in no folder gets one of its own.
            let folder = directory ?? "/chats/\(id)"
            let session = Session(
                id: id, name: request.name, backend: request.backend,
                workingDirectory: folder, model: request.model, host: request.host,
                approvalMode: request.approvalMode, projectID: projectID, itemID: request.itemID)
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
    private static func copy(of project: Project, for session: String) -> String {
        let folder = "/copies/\(session)"
        guard project.repositories.count == 1, let repository = project.repositories.first else { return folder }
        return "\(folder)/\(repository.name)"
    }

    func send(_ session: String, _ text: String, _ mode: SendMode) throws {
        try known(session)
        if state.value.sessions.first(where: { $0.id == session })?.archived == true {
            throw DaemonError.refused(
                .archived, message: "Session \(session) is archived; unarchive it to work in it again.")
        }
        let busy = state.withValue { state -> Bool in
            guard state.paused[session] != nil else { return false }
            if mode == .enqueue { state.queues[session, default: []].append(text) }
            return true
        }
        if !busy {
            play(session, text)
        } else if mode == .steer {
            // A turn waiting on a person reads the steer when it resumes.
            append(.userMessage(text), to: session)
        }
    }

    /// Plays turns for `text`, then for each queued message, until one
    /// waits for permission or the queue is empty.
    private func play(_ session: String, _ text: String) {
        var next: String? = text
        while let text = next {
            append(.userMessage(text), to: session)
            append(.turnStarted, to: session)
            guard proceed(session, steps: script(text)) else { return }
            next = state.withValue { state in
                state.queues[session]?.isEmpty == false ? state.queues[session]?.removeFirst() : nil
            }
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
        case .say, .think: return
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
        let next = state.withValue { state in
            state.queues[session]?.isEmpty == false ? state.queues[session]?.removeFirst() : nil
        }
        if let next { play(session, next) }
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
        let next = state.withValue { state in
            state.queues[session]?.isEmpty == false ? state.queues[session]?.removeFirst() : nil
        }
        if let next {
            play(session, next)
        } else {
            append(.idle, to: session)
        }
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
                if kind.endsTurn { state.sessions[index].turns += 1 }
                switch kind {
                case .turnStarted, .turnEnded: state.sessions[index].pending = []
                case .permissionRequested(let request, let permission, let summary):
                    state.sessions[index].pending.append(
                        PendingPermission(requestID: request, kind: permission, summary: summary))
                case .permissionCompleted(let request, _, _):
                    state.sessions[index].pending.removeAll { $0.requestID == request }
                default: break
                }
            }
            return (event, Array((state.watchers[session] ?? [:]).values))
        }
        for watcher in watchers {
            watcher.yield(event)
        }
        return event.id
    }

    private static func state(after kind: SessionEvent.Kind, waiting: Bool) -> SessionState {
        switch kind {
        case .idle, .turnEnded: .idle
        case .permissionRequested: .waitingForApproval
        default: waiting ? .waitingForApproval : .working
        }
    }

    private func fork(_ session: String, before: String?, name: String?, chatOnly: Bool) throws -> Session {
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

    private func watch(_ session: String) -> AsyncThrowingStream<SessionEvent, any Error> {
        AsyncThrowingStream { continuation in
            let id = UUID()
            let known = state.withValue { state -> Bool in
                guard let history = state.histories[session] else { return false }
                for event in history {
                    continuation.yield(event)
                }
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
    private static func unknownItem(_ item: String) -> DaemonError {
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
            let shown = Self.show(items, sessions: state.sessions)
            return ordered(shown.filter { $0.parentID == nil }).flatMap { task in
                [task] + ordered(shown.filter { $0.parentID == task.id })
            }
        }
    }

    /// What a session says about its item: waiting or failed is Waiting on
    /// you, working is In progress, a finished turn is In review.
    private static func said(by session: Session) -> ItemStatus? {
        guard !session.archived else { return nil }
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

    private func createItem(_ project: String, _ fields: ItemFields) throws -> Item {
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
            Self.log(&state, item: item.id, before: nil, after: fields, time: time)
            return item
        }
    }

    private func updateItem(_ itemID: String, _ update: ItemUpdate) throws -> Item {
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
                Self.log(&state, item: itemID, before: before, after: after, time: time)
            }
            return state.items[index].projectID
        }
        return try shown(itemID, in: projectID)
    }

    private func shown(_ itemID: String, in project: String) throws -> Item {
        guard let item = try items(project).first(where: { $0.id == itemID }) else { throw Self.unknownItem(itemID) }
        return item
    }

    private static func log(
        _ state: inout State, item: String, before: ItemFields?, after: ItemFields?, time: Date, reverts: UInt64? = nil
    ) {
        state.changes.append(
            BoardChange(id: state.nextChange, itemID: item, time: time, before: before, after: after, reverts: reverts))
        state.nextChange += 1
    }

    /// Puts the item back as it was before the change, removing it if the
    /// change added it.
    private func revert(_ changeID: UInt64) throws -> Item? {
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
                Self.log(&state, item: item.id, before: item.fields, after: nil, time: time, reverts: changeID)
                return nil
            }
            let projectID: String
            if let index {
                let current = state.items[index].fields
                state.items[index].fields = before
                state.items[index].updated = time
                projectID = state.items[index].projectID
                Self.log(&state, item: change.itemID, before: current, after: before, time: time, reverts: changeID)
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
                Self.log(&state, item: change.itemID, before: nil, after: before, time: time, reverts: changeID)
            }
            return (change.itemID, projectID)
        }
        guard let (itemID, projectID) = kept else { return nil }
        return try shown(itemID, in: projectID)
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

    /// Sessions waiting on you: approvals, then failures, then replies.
    private func inbox() -> [InboxEntry] {
        state.withValue { state in
            let entries = state.sessions.compactMap { session -> InboxEntry? in
                guard !session.archived else { return nil }
                let item = state.items.first { $0.id == session.itemID }
                let reason: InboxReason
                switch session.state {
                case .waitingForApproval: reason = .approval
                case .failed: reason = .failed
                case .idle where session.turns > session.seenTurns && item?.fields.status.closed != true:
                    reason = .reply
                case .idle, .working: return nil
                }
                return InboxEntry(
                    session: session, reason: reason,
                    projectName: state.projects.first { $0.id == session.projectID }?.name, itemTitle: item?.title)
            }
            let order: [InboxReason] = [.approval, .failed, .reply]
            return order.flatMap { reason in entries.filter { $0.reason == reason } }
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
        case .say, .think: nil
        }
    }
}
