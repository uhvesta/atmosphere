import ComposableArchitecture
import Foundation

/// A daemon kept in memory, for tests and previews. Each message plays the
/// steps `script(text)` gives, at once; a step that needs permission waits
/// for an answer unless the session's approval mode allows it. Ids count up
/// (`p1`, `s1`, `e1`, `r1`, ...) and times from `start`, so tests see the
/// same values every run.
package final class InMemoryDaemon: Sendable {
    /// What the agent does in reply to a message.
    package enum Step: Hashable, Sendable {
        case say(String)
        case think(String)
        /// Asks to run `command`; if allowed, reports `output`.
        case run(command: String, output: String)
        /// Asks to write `contents` to `path`; if allowed, writes it to
        /// ``files``.
        case write(path: String, contents: String)
    }

    /// A turn stopped at a permission request, with the steps after it.
    private struct Paused {
        var request: String
        var step: Step
        var callID: String
        var rest: [Step]
    }

    private struct State {
        var projects: [Project] = []
        var sessions: [Session] = []
        var histories: [String: [SessionEvent]] = [:]
        var paused: [String: Paused] = [:]
        var queues: [String: [String]] = [:]
        var files: [String: String] = [:]
        var watchers: [String: [UUID: AsyncThrowingStream<SessionEvent, any Error>.Continuation]] = [:]
        var nextProject = 1
        var nextSession = 1
        var nextEvent = 1
        var nextRequest = 1
    }

    private let state = LockIsolated(State())
    private let script: @Sendable (String) -> [Step]
    private let start: Date

    package init(
        start: Date = Date(timeIntervalSince1970: 1_794_000_000),
        script: @escaping @Sendable (String) -> [Step] = { [.say("You said: \($0)")] }
    ) {
        self.start = start
        self.script = script
    }

    /// The sessions as they are now.
    package var sessions: [Session] { state.value.sessions }

    /// What the agent has written, by path.
    package var files: [String: String] { state.value.files }

    package func history(_ session: String) -> [SessionEvent] {
        state.value.histories[session] ?? []
    }

    package var client: DaemonClient {
        DaemonClient(
            projects: { self.state.value.projects },
            createProject: { name, repository, instructions in
                try self.createProject(name: name, repository: repository, instructions: instructions)
            },
            sessions: { project in
                self.state.value.sessions.filter { project == nil || $0.projectID == project }
            },
            start: { request in try self.startSession(request) },
            send: { session, text, mode in try self.send(session, text, mode) },
            approve: { session, request, approved in try self.answer(session, request, approved: approved) },
            setApprovalMode: { session, mode in try self.setApprovalMode(session, mode) },
            abort: { session in try self.abort(session) },
            fork: { session, before, name in try self.fork(session, before: before, name: name) },
            history: { session in
                try self.known(session)
                return self.history(session)
            },
            watch: { session in self.watch(session) }
        )
    }

    private static func unknown(_ session: String) -> DaemonError {
        .refused(.unknownSession, message: "There is no session \(session); list sessions to find its id.")
    }

    private func known(_ session: String) throws {
        guard state.value.histories[session] != nil else { throw Self.unknown(session) }
    }

    private func createProject(name: String, repository: String, instructions: String) throws -> Project {
        guard repository.hasPrefix("/") else {
            throw DaemonError.refused(
                .invalidRequest,
                message: "The repository \(repository) must be an absolute path to a folder on this Mac.")
        }
        return state.withValue { state in
            let project = Project(
                id: "p\(state.nextProject)", name: name, repository: repository, instructions: instructions,
                created: start + Double(state.nextProject))
            state.nextProject += 1
            state.projects.append(project)
            return project
        }
    }

    private func startSession(_ request: StartRequest) throws -> Session {
        try state.withValue { state in
            var directory = request.workingDirectory
            if let projectID = request.projectID {
                guard let project = state.projects.first(where: { $0.id == projectID }) else {
                    throw DaemonError.refused(.unknownProject, message: "There is no project \(projectID).")
                }
                directory = directory ?? project.repository
            }
            guard let directory else {
                throw DaemonError.refused(.invalidRequest, message: "A session needs a working directory.")
            }
            let session = Session(
                id: "s\(state.nextSession)", name: request.name, backend: request.backend,
                workingDirectory: directory, model: request.model, host: request.host,
                approvalMode: request.approvalMode, projectID: request.projectID)
            state.nextSession += 1
            state.sessions.append(session)
            state.histories[session.id] = []
            return session
        }
    }

    private func send(_ session: String, _ text: String, _ mode: SendMode) throws {
        try known(session)
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
        case .write(let path, let contents):
            if approved { state.withValue { $0.files[path] = contents } }
            output = approved ? "Wrote \(path)" : "The person denied this."
        case .say, .think: return
        }
        append(.toolCompleted(callID: callID, success: approved, output: output), to: session)
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

    private func append(_ kind: SessionEvent.Kind, to session: String) {
        let (event, watchers) = state.withValue { state in
            let event = SessionEvent(id: "e\(state.nextEvent)", time: start + Double(state.nextEvent), kind: kind)
            state.nextEvent += 1
            state.histories[session, default: []].append(event)
            if let index = state.sessions.firstIndex(where: { $0.id == session }) {
                state.sessions[index].state = Self.state(after: kind, waiting: state.paused[session] != nil)
            }
            return (event, Array((state.watchers[session] ?? [:]).values))
        }
        for watcher in watchers {
            watcher.yield(event)
        }
    }

    private static func state(after kind: SessionEvent.Kind, waiting: Bool) -> SessionState {
        switch kind {
        case .idle, .turnEnded: .idle
        case .permissionRequested: .waitingForApproval
        default: waiting ? .waitingForApproval : .working
        }
    }

    private func fork(_ session: String, before: String?, name: String?) throws -> Session {
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
            state.nextSession += 1
            state.sessions.append(fork)
            state.histories[fork.id] = history
            return fork
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
