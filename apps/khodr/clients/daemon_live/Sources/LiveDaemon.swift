import Foundation
import KhodrDaemonClient
import KhodrProto

extension DaemonClient {
    /// Where khodrd listens unless told otherwise: `$KHODR_SOCKET`, else
    /// `~/Library/Application Support/Khodr/daemon.sock`, as khodrd itself
    /// decides.
    package static var defaultSocket: String {
        if let path = ProcessInfo.processInfo.environment["KHODR_SOCKET"], !path.isEmpty { return path }
        return FileManager.default.homeDirectoryForCurrentUser
            .appending(path: "Library/Application Support/Khodr/daemon.sock").path
    }

    /// Talks to khodrd on `socket`, connecting on first use and again after
    /// the connection drops.
    package static func live(socket: String = defaultSocket, programVersion: String = "0.0.0") -> DaemonClient {
        let daemon = LiveDaemon(socket: socket, programVersion: programVersion)
        return DaemonClient(
            projects: {
                guard case .projects(let projects) = try await daemon.call(.listProjects(Khodr_V1_ListProjects()))
                else {
                    throw DaemonError.unexpected("something other than the projects")
                }
                return projects.projects.map(Project.init)
            },
            createProject: { name, repository, instructions in
                var create = Khodr_V1_CreateProject()
                create.name = name
                create.repository = repository
                create.instructions = instructions
                guard case .projectCreated(let created) = try await daemon.call(.createProject(create)) else {
                    throw DaemonError.unexpected("something other than the created project")
                }
                return Project(created.project)
            },
            sessions: { project in
                var list = Khodr_V1_ListSessions()
                list.projectID = project ?? ""
                guard case .sessions(let sessions) = try await daemon.call(.listSessions(list)) else {
                    throw DaemonError.unexpected("something other than the sessions")
                }
                return try sessions.sessions.map(Session.init)
            },
            start: { request in
                var start = Khodr_V1_StartSession()
                start.backend = request.backend.proto
                start.workingDirectory = request.workingDirectory ?? ""
                start.approvalMode = request.approvalMode.proto
                start.projectID = request.projectID ?? ""
                start.name = request.name
                start.host = request.host ?? ""
                start.model = request.model ?? ""
                start.instructions = request.instructions ?? ""
                start.backendConfig = request.backendConfig ?? ""
                guard case .sessionStarted(let started) = try await daemon.call(.startSession(start)) else {
                    throw DaemonError.unexpected("something other than the started session")
                }
                return try Session(started.session)
            },
            send: { session, text, mode in
                var message = Khodr_V1_SendMessage()
                message.sessionID = session
                message.text = text
                message.mode = mode == .steer ? .immediate : .enqueue
                _ = try await daemon.call(.sendMessage(message))
            },
            approve: { session, request, approved in
                var approve = Khodr_V1_Approve()
                approve.sessionID = session
                approve.permissionRequestID = request
                approve.approved = approved
                _ = try await daemon.call(.approve(approve))
            },
            setApprovalMode: { session, mode in
                var set = Khodr_V1_SetApprovalMode()
                set.sessionID = session
                set.mode = mode.proto
                _ = try await daemon.call(.setApprovalMode(set))
            },
            abort: { session in
                var abort = Khodr_V1_AbortTurn()
                abort.sessionID = session
                _ = try await daemon.call(.abortTurn(abort))
            },
            fork: { session, before, name in
                var fork = Khodr_V1_ForkSession()
                fork.sessionID = session
                fork.beforeEventID = before ?? ""
                fork.name = name ?? ""
                guard case .sessionForked(let forked) = try await daemon.call(.forkSession(fork)) else {
                    throw DaemonError.unexpected("something other than the forked session")
                }
                return try Session(forked.session)
            },
            history: { session in
                var read = Khodr_V1_ReadHistory()
                read.sessionID = session
                guard case .history(let history) = try await daemon.call(.readHistory(read)) else {
                    throw DaemonError.unexpected("something other than the history")
                }
                return history.events.compactMap(SessionEvent.init)
            },
            watch: { session in
                AsyncThrowingStream { continuation in
                    let task = Task {
                        do {
                            var watch = Khodr_V1_WatchSession()
                            watch.sessionID = session
                            for try await item in try await daemon.stream(.watchSession(watch)) {
                                if case .sessionEvent(let event) = item.kind, let event = SessionEvent(event) {
                                    continuation.yield(event)
                                }
                            }
                            continuation.finish()
                        } catch {
                            continuation.finish(throwing: error)
                        }
                    }
                    continuation.onTermination = { _ in task.cancel() }
                }
            }
        )
    }
}

/// Keeps one connection to khodrd, replacing it once it closes.
private actor LiveDaemon {
    private let socket: String
    private let programVersion: String
    private var connection: Connection?

    init(socket: String, programVersion: String) {
        self.socket = socket
        self.programVersion = programVersion
    }

    private func connected() async throws -> Connection {
        if let connection, !connection.isClosed { return connection }
        let opened = try await Connection.open(path: socket, programVersion: programVersion)
        connection = opened
        return opened
    }

    func call(_ request: Khodr_V1_Request.OneOf_Kind) async throws -> Khodr_V1_Response.OneOf_Kind {
        try await connected().call(request)
    }

    func stream(_ request: Khodr_V1_Request.OneOf_Kind) async throws -> Items {
        try await connected().stream(request)
    }
}
