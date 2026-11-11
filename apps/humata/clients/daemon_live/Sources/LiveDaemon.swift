import Foundation
import HumataDaemonClient
import HumataProto

extension DaemonClient {
    /// Where humatad listens unless told otherwise: `$HUMATA_SOCKET`, else
    /// `~/Library/Application Support/Humata/daemon.sock`, as humatad itself
    /// decides.
    package static var defaultSocket: String {
        if let path = ProcessInfo.processInfo.environment["HUMATA_SOCKET"], !path.isEmpty { return path }
        return FileManager.default.homeDirectoryForCurrentUser
            .appending(path: "Library/Application Support/Humata/daemon.sock").path
    }

    /// What the daemon on `socket` says it is in its hello, such as
    /// `0.0.0+1a2b3c4d5e6f7a8b`: its version and the start of its
    /// executable's SHA-256. Throws `DaemonError.versionMismatch` if it
    /// speaks another protocol.
    package static func daemonBuild(socket: String = defaultSocket, programVersion: String = "0.0.0")
        async throws -> String
    {
        let connection = try await Connection.open(path: socket, programVersion: programVersion)
        defer { connection.close() }
        return connection.peer.programVersion
    }

    /// Talks to humatad on `socket`, connecting on first use and again after
    /// the connection drops.
    package static func live(socket: String = defaultSocket, programVersion: String = "0.0.0") -> DaemonClient {
        let daemon = LiveDaemon(socket: socket, programVersion: programVersion)
        return DaemonClient(
            projects: {
                guard case .projects(let projects) = try await daemon.call(.listProjects(Humata_V1_ListProjects()))
                else {
                    throw DaemonError.unexpected("something other than the projects")
                }
                return projects.projects.map(Project.init)
            },
            createProject: { name, repositories, instructions in
                var create = Humata_V1_CreateProject()
                create.name = name
                create.repositories = repositories.map(\.proto)
                create.instructions = instructions
                guard case .projectCreated(let created) = try await daemon.call(.createProject(create)) else {
                    throw DaemonError.unexpected("something other than the created project")
                }
                return Project(created.project)
            },
            sessions: { project in
                var list = Humata_V1_ListSessions()
                list.projectID = project ?? ""
                guard case .sessions(let sessions) = try await daemon.call(.listSessions(list)) else {
                    throw DaemonError.unexpected("something other than the sessions")
                }
                return try sessions.sessions.map(Session.init)
            },
            start: { request in
                var start = Humata_V1_StartSession()
                start.backend = request.backend.proto
                start.workingDirectory = request.workingDirectory ?? ""
                start.approvalMode = request.approvalMode.proto
                start.projectID = request.projectID ?? ""
                start.name = request.name
                start.host = request.host ?? ""
                start.model = request.model ?? ""
                start.instructions = request.instructions ?? ""
                start.backendConfig = request.backendConfig ?? ""
                start.itemID = request.itemID ?? ""
                start.specs = request.specs == .withParents ? .withParents : .item
                guard case .sessionStarted(let started) = try await daemon.call(.startSession(start)) else {
                    throw DaemonError.unexpected("something other than the started session")
                }
                return try Session(started.session)
            },
            send: { session, text, mode in
                var message = Humata_V1_SendMessage()
                message.sessionID = session
                message.text = text
                message.mode = mode == .steer ? .immediate : .enqueue
                _ = try await daemon.call(.sendMessage(message))
            },
            approve: { session, request, approved in
                var approve = Humata_V1_Approve()
                approve.sessionID = session
                approve.permissionRequestID = request
                approve.approved = approved
                _ = try await daemon.call(.approve(approve))
            },
            setApprovalMode: { session, mode in
                var set = Humata_V1_SetApprovalMode()
                set.sessionID = session
                set.mode = mode.proto
                _ = try await daemon.call(.setApprovalMode(set))
            },
            abort: { session in
                var abort = Humata_V1_AbortTurn()
                abort.sessionID = session
                _ = try await daemon.call(.abortTurn(abort))
            },
            fork: { session, before, name, chatOnly in
                var fork = Humata_V1_ForkSession()
                fork.sessionID = session
                fork.beforeEventID = before ?? ""
                fork.name = name ?? ""
                fork.chatOnly = chatOnly
                guard case .sessionForked(let forked) = try await daemon.call(.forkSession(fork)) else {
                    throw DaemonError.unexpected("something other than the forked session")
                }
                return try Session(forked.session)
            },
            update: { session, update in
                var request = Humata_V1_UpdateSession()
                request.sessionID = session
                if let name = update.name { request.name = name }
                if let archived = update.archived { request.archived = archived }
                if let keeper = update.keeper { request.keeper = keeper }
                switch update.placement {
                case .item(let item): request.itemID = item
                case .project: request.itemID = ""
                case nil: break
                }
                guard case .sessionUpdated(let updated) = try await daemon.call(.updateSession(request)) else {
                    throw DaemonError.unexpected("something other than the updated session")
                }
                return try Session(updated.session)
            },
            changes: { session, query in
                guard case .changes(let changes) = try await daemon.call(.readChanges(query.proto(session))) else {
                    throw DaemonError.unexpected("something other than the session's changes")
                }
                return changes.repositories.map(RepositoryChanges.init)
            },
            editChanges: { session, edit in
                var request = Humata_V1_EditChanges()
                request.sessionID = session
                request.file = edit.file.proto
                request.action = edit.action.proto
                if let hunk = edit.hunk {
                    request.hunk.oldStart = UInt32(hunk.oldStart)
                    request.hunk.newStart = UInt32(hunk.newStart)
                    request.hunk.context = UInt32(hunk.context)
                }
                _ = try await daemon.call(.editChanges(request))
            },
            markViewed: { session, file, blob, viewed in
                var mark = Humata_V1_MarkViewed()
                mark.sessionID = session
                mark.file = file.proto
                mark.blob = blob
                mark.viewed = viewed
                _ = try await daemon.call(.markViewed(mark))
            },
            createThread: { session, thread in
                var create = Humata_V1_CreateThread()
                create.sessionID = session
                create.file = thread.file.proto
                create.startLine = UInt32(thread.startLine)
                create.endLine = UInt32(thread.endLine)
                create.quote = thread.quote
                create.text = thread.text
                return try ReviewThread(saved: await daemon.call(.createThread(create)))
            },
            replyToThread: { thread, text in
                var reply = Humata_V1_ReplyToThread()
                reply.threadID = thread
                reply.text = text
                return try ReviewThread(saved: await daemon.call(.replyToThread(reply)))
            },
            resolveThread: { thread, resolved in
                var resolve = Humata_V1_ResolveThread()
                resolve.threadID = thread
                resolve.resolved = resolved
                return try ReviewThread(saved: await daemon.call(.resolveThread(resolve)))
            },
            deleteComment: { comment in
                var delete = Humata_V1_DeleteComment()
                delete.commentID = comment
                _ = try await daemon.call(.deleteComment(delete))
            },
            threads: { session in
                var list = Humata_V1_ListThreads()
                list.sessionID = session
                guard case .threads(let threads) = try await daemon.call(.listThreads(list)) else {
                    throw DaemonError.unexpected("something other than the review's threads")
                }
                return threads.threads.map(ReviewThread.init)
            },
            sendReview: { session in
                var send = Humata_V1_SendReview()
                send.sessionID = session
                guard case .reviewSent(let sent) = try await daemon.call(.sendReview(send)) else {
                    throw DaemonError.unexpected("something other than the sent review")
                }
                return Int(sent.comments)
            },
            files: { session in
                var list = Humata_V1_ListFiles()
                list.sessionID = session
                guard case .files(let files) = try await daemon.call(.listFiles(list)) else {
                    throw DaemonError.unexpected("something other than the session's files")
                }
                return ChatFiles(files: files.files.map(ChatFile.init), truncated: files.truncated)
            },
            readFile: { session, path in
                var read = Humata_V1_ReadFile()
                read.sessionID = session
                read.path = path
                guard case .fileContents(let contents) = try await daemon.call(.readFile(read)) else {
                    throw DaemonError.unexpected("something other than the file's contents")
                }
                return FileContents(path: contents.path, text: contents.binary ? nil : contents.text)
            },
            items: { project in
                var list = Humata_V1_ListItems()
                list.projectID = project
                guard case .items(let items) = try await daemon.call(.listItems(list)) else {
                    throw DaemonError.unexpected("something other than the board's items")
                }
                return items.items.map(Item.init)
            },
            createItem: { project, fields in
                var create = Humata_V1_CreateItem()
                create.projectID = project
                create.parentID = fields.parentID ?? ""
                create.title = fields.title
                create.spec = fields.spec
                create.status = fields.status.proto
                create.priority = fields.priority.proto
                guard case .itemCreated(let created) = try await daemon.call(.createItem(create)) else {
                    throw DaemonError.unexpected("something other than the created item")
                }
                return Item(created.item)
            },
            updateItem: { item, update in
                var request = Humata_V1_UpdateItem()
                request.itemID = item
                if let title = update.title { request.title = title }
                if let spec = update.spec { request.spec = spec }
                if let status = update.status { request.status = status.proto }
                if let priority = update.priority { request.priority = priority.proto }
                guard case .itemUpdated(let updated) = try await daemon.call(.updateItem(request)) else {
                    throw DaemonError.unexpected("something other than the updated item")
                }
                return Item(updated.item)
            },
            boardLog: { project in
                var read = Humata_V1_ReadBoardLog()
                read.projectID = project
                guard case .boardLog(let log) = try await daemon.call(.readBoardLog(read)) else {
                    throw DaemonError.unexpected("something other than the board's changes")
                }
                return log.changes.map(BoardChange.init)
            },
            revert: { change in
                var revert = Humata_V1_RevertBoardChange()
                revert.changeID = change
                guard case .boardChangeReverted(let reverted) = try await daemon.call(.revertBoardChange(revert))
                else {
                    throw DaemonError.unexpected("something other than the reverted item")
                }
                return reverted.hasItem ? Item(reverted.item) : nil
            },
            inbox: {
                guard case .inbox(let inbox) = try await daemon.call(.listInbox(Humata_V1_ListInbox())) else {
                    throw DaemonError.unexpected("something other than the inbox")
                }
                return try inbox.entries.compactMap(InboxEntry.init)
            },
            history: { session in
                var read = Humata_V1_ReadHistory()
                read.sessionID = session
                guard case .history(let history) = try await daemon.call(.readHistory(read)) else {
                    throw DaemonError.unexpected("something other than the history")
                }
                return history.events.compactMap(SessionEvent.init)
            },
            hosts: {
                guard case .hosts(let hosts) = try await daemon.call(.listHosts(Humata_V1_ListHosts())) else {
                    throw DaemonError.unexpected("something other than the hosts")
                }
                return hosts.hosts.map(RemoteHost.init)
            },
            addHost: { name, destination in
                var add = Humata_V1_AddHost()
                add.name = name
                add.sshDestination = destination
                guard case .hostSaved(let saved) = try await daemon.call(.addHost(add)) else {
                    throw DaemonError.unexpected("something other than the added host")
                }
                return RemoteHost(saved.host)
            },
            updateHost: { host, update in
                var request = Humata_V1_UpdateHost()
                request.name = host
                request.setEnv = update.setEnv.sorted { $0.key < $1.key }.map { name, value in
                    var variable = Humata_V1_EnvVar()
                    variable.name = name
                    variable.value = value
                    return variable
                }
                request.unsetEnv = update.unsetEnv
                if let script = update.setupScript { request.setupScript = script }
                guard case .hostSaved(let saved) = try await daemon.call(.updateHost(request)) else {
                    throw DaemonError.unexpected("something other than the updated host")
                }
                return RemoteHost(saved.host)
            },
            removeHost: { host in
                var remove = Humata_V1_RemoveHost()
                remove.name = host
                _ = try await daemon.call(.removeHost(remove))
            },
            checkHost: { host in
                var check = Humata_V1_CheckHost()
                check.name = host
                guard case .hostHealth(let health) = try await daemon.call(.checkHost(check)) else {
                    throw DaemonError.unexpected("something other than the host's health")
                }
                return HostHealth(health)
            },
            installHost: { host in
                var install = Humata_V1_InstallHost()
                install.name = host
                guard case .hostInstalled(let installed) = try await daemon.call(.installHost(install)) else {
                    throw DaemonError.unexpected("something other than the installed version")
                }
                return installed.programVersion
            },
            sshHosts: {
                guard case .sshHosts(let hosts) = try await daemon.call(.listSshHosts(Humata_V1_ListSshHosts())) else {
                    throw DaemonError.unexpected("something other than the SSH config's hosts")
                }
                return hosts.names
            },
            watch: { session in
                AsyncThrowingStream { continuation in
                    let task = Task {
                        do {
                            var watch = Humata_V1_WatchSession()
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

/// Keeps one connection to humatad, replacing it once it closes.
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

    func call(_ request: Humata_V1_Request.OneOf_Kind) async throws -> Humata_V1_Response.OneOf_Kind {
        try await connected().call(request)
    }

    func stream(_ request: Humata_V1_Request.OneOf_Kind) async throws -> Items {
        try await connected().stream(request)
    }
}
