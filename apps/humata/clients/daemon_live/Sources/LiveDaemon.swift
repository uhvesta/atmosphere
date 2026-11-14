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
        let unanswered = UnansweredSends()
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
            deleteProject: { project in
                var delete = Humata_V1_DeleteProject()
                delete.projectID = project
                guard case .projectDeleted = try await daemon.call(.deleteProject(delete)) else {
                    throw DaemonError.unexpected("something other than the deleted project")
                }
            },
            updateProject: { project, update in
                var request = Humata_V1_UpdateProject()
                request.projectID = project
                if let name = update.name { request.name = name }
                if let instructions = update.instructions { request.instructions = instructions }
                if let skills = update.skills { request.skills.skills = skills.map(\.proto) }
                if let repositories = update.repositories {
                    request.repositories.repositories = repositories.map(\.proto)
                }
                if let agentSessions = update.agentSessions { request.agentSessions = agentSessions.proto }
                guard case .projectUpdated(let updated) = try await daemon.call(.updateProject(request)) else {
                    throw DaemonError.unexpected("something other than the updated project")
                }
                return Project(updated.project)
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
                start.effort = request.effort ?? ""
                start.experimental = request.experimental
                start.instructions = request.instructions ?? ""
                start.backendConfig = request.backendConfig ?? ""
                start.itemID = request.itemID ?? ""
                start.planning = request.planning
                start.specs = request.specs == .withParents ? .withParents : .item
                guard case .sessionStarted(let started) = try await daemon.call(.startSession(start)) else {
                    throw DaemonError.unexpected("something other than the started session")
                }
                return try Session(started.session)
            },
            importable: { host in
                var list = Humata_V1_ListImportable()
                list.host = host ?? ""
                guard case .importable(let found) = try await daemon.call(.listImportable(list)) else {
                    throw DaemonError.unexpected("something other than the sessions to import")
                }
                return try found.sessions.map(ImportableSession.init)
            },
            importSession: { session, host, projectID in
                var request = Humata_V1_ImportSession()
                request.backend = session.backend.proto
                request.externalID = session.id
                request.host = host ?? ""
                request.projectID = projectID ?? ""
                guard case .sessionStarted(let started) = try await daemon.call(.importSession(request)) else {
                    throw DaemonError.unexpected("something other than the imported session")
                }
                return try Session(started.session)
            },
            models: { host, backend, backendConfig in
                var list = Humata_V1_ListModels()
                list.host = host ?? ""
                list.backend = backend.proto
                list.backendConfig = backendConfig ?? ""
                guard case .models(let found) = try await daemon.call(.listModels(list)) else {
                    throw DaemonError.unexpected("something other than the models")
                }
                return found.models.map {
                    ModelChoice(
                        id: $0.id, name: $0.name.isEmpty ? $0.id : $0.name, detail: $0.detail.isEmpty ? nil : $0.detail,
                        readsImages: $0.readsImages, efforts: $0.efforts,
                        defaultEffort: $0.defaultEffort.isEmpty ? nil : $0.defaultEffort)
                }
            },
            skipped: { host in
                var list = Humata_V1_ListSkipped()
                list.host = host ?? ""
                guard case .skipped(let found) = try await daemon.call(.listSkipped(list)) else {
                    throw DaemonError.unexpected("something other than the skipped sessions")
                }
                return try found.sessions.map(SkippedSession.init)
            },
            removeSkipped: { session, host in
                var remove = Humata_V1_RemoveSkipped()
                remove.host = host ?? ""
                remove.backend = session.backend.proto
                remove.id = session.id
                guard case .skippedRemoved = try await daemon.call(.removeSkipped(remove)) else {
                    throw DaemonError.unexpected("something other than the skipped session removed")
                }
            },
            send: { session, text, mode in
                var message = Humata_V1_SendMessage()
                message.sessionID = session
                message.text = text
                message.mode = mode == .steer ? .immediate : .enqueue
                // Sending the same text again soon after a send that got no
                // answer reuses its id, so a host that took it runs it once.
                message.clientID = unanswered.id(session: session, text: text, at: Date())
                // A host that refused it forgot the id, so keeping it is safe.
                _ = try await daemon.call(.sendMessage(message))
                unanswered.answered(session: session)
            },
            sendImages: { session, text, images, mode in
                var message = Humata_V1_SendMessage()
                message.sessionID = session
                message.text = text
                message.mode = mode == .steer ? .immediate : .enqueue
                message.images = images.map { image in
                    var sent = Humata_V1_Image()
                    sent.mediaType = image.mediaType
                    sent.data = image.data
                    return sent
                }
                // Images are too large to send twice: a lost answer is
                // sent again as new, which the person sees and can remove.
                _ = try await daemon.call(.sendMessage(message))
            },
            readImage: { session, name in
                var read = Humata_V1_ReadImage()
                read.sessionID = session
                read.name = name
                guard case .imageData(let found) = try await daemon.call(.readImage(read)) else {
                    throw DaemonError.unexpected("something other than the image")
                }
                return ImageAttachment(mediaType: found.image.mediaType, data: found.image.data)
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
            stopAll: { session in
                var abort = Humata_V1_AbortTurn()
                abort.sessionID = session
                abort.dropQueued = true
                _ = try await daemon.call(.abortTurn(abort))
            },
            rewind: { session, before, files in
                var rewind = Humata_V1_RewindSession()
                rewind.sessionID = session
                rewind.beforeEventID = before
                rewind.files = files
                _ = try await daemon.call(.rewindSession(rewind))
            },
            undoRewind: { session in
                var undo = Humata_V1_UndoRewind()
                undo.sessionID = session
                _ = try await daemon.call(.undoRewind(undo))
            },
            compact: { session, focus, stop in
                var compact = Humata_V1_CompactSession()
                compact.sessionID = session
                compact.focus = focus
                compact.stop = stop
                guard case .compactStarted = try await daemon.call(.compactSession(compact)) else {
                    throw DaemonError.unexpected("something other than the compaction having started")
                }
            },
            chatCommands: { session in
                var get = Humata_V1_GetChatCommands()
                get.sessionID = session
                guard case .chatCommands(let commands) = try await daemon.call(.getChatCommands(get)) else {
                    throw DaemonError.unexpected("something other than the session's chat commands")
                }
                return ChatCommands(commands)
            },
            unqueue: { session, queueID in
                var unqueue = Humata_V1_Unqueue()
                unqueue.sessionID = session
                unqueue.queueID = queueID
                _ = try await daemon.call(.unqueue(unqueue))
            },
            editQueued: { session, queueID, text in
                var edit = Humata_V1_EditQueued()
                edit.sessionID = session
                edit.queueID = queueID
                edit.text = text
                _ = try await daemon.call(.editQueued(edit))
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
            newChild: { session, name, kind in
                var fork = Humata_V1_ForkSession()
                fork.sessionID = session
                fork.name = name
                switch kind {
                case .empty(let ownCopy):
                    fork.empty = true
                    fork.chatOnly = !ownCopy
                case .side:
                    fork.readOnly = true
                    fork.chatOnly = true
                }
                guard case .sessionForked(let forked) = try await daemon.call(.forkSession(fork)) else {
                    throw DaemonError.unexpected("something other than the new chat")
                }
                let child = try Session(forked.session)
                // A host from before side chats makes a fork that can
                // write: it mustn't pass for one that can't.
                if kind == .side, !child.readOnly {
                    throw DaemonError.refused(
                        .invalidRequest,
                        message: "The session's host runs an older Humata that can't hold a side chat to reading "
                            + "only, so what it made, “\(child.name)” in the sidebar, is an ordinary fork that "
                            + "can change files. Archive that fork, update Humata on the host, then start the "
                            + "side chat again.")
                }
                // One from before empty chats makes a fork with the whole
                // conversation: it mustn't pass for a chat that starts anew.
                if case .empty = kind, child.turns > 0 {
                    throw DaemonError.refused(
                        .invalidRequest,
                        message: "The session's host runs an older Humata that can't start a chat with none of "
                            + "this one in it, so what it made, “\(child.name)” in the sidebar, is a fork with the "
                            + "whole conversation, on this session's copy of the code. Archive that fork, update "
                            + "Humata on the host, then start the chat again.")
                }
                return child
            },
            startWork: { session, item in
                var fork = Humata_V1_ForkSession()
                fork.sessionID = session
                fork.itemID = item
                guard case .sessionForked(let forked) = try await daemon.call(.forkSession(fork)) else {
                    throw DaemonError.unexpected("something other than the forked session")
                }
                return try Session(forked.session)
            },
            proposals: { session in
                var list = Humata_V1_ListProposals()
                list.sessionID = session
                guard case .proposals(let proposals) = try await daemon.call(.listProposals(list)) else {
                    throw DaemonError.unexpected("something other than the proposals")
                }
                return proposals.proposals.map(Proposal.init)
            },
            outsideProposals: { project in
                var list = Humata_V1_ListProposals()
                list.projectID = project
                guard case .proposals(let proposals) = try await daemon.call(.listProposals(list)) else {
                    throw DaemonError.unexpected("something other than the proposals")
                }
                return proposals.proposals.map(Proposal.init)
            },
            applyProposal: { session, proposal, title, spec in
                var apply = Humata_V1_ApplyProposal()
                apply.sessionID = session
                apply.proposalID = proposal
                if let title { apply.title = title }
                if let spec { apply.spec = spec }
                guard case .proposalApplied(let applied) = try await daemon.call(.applyProposal(apply)) else {
                    throw DaemonError.unexpected("something other than the applied proposal")
                }
                return Item(applied.item)
            },
            declineProposal: { session, proposal in
                var decline = Humata_V1_DeclineProposal()
                decline.sessionID = session
                decline.proposalID = proposal
                guard case .proposalDeclined = try await daemon.call(.declineProposal(decline)) else {
                    throw DaemonError.unexpected("something other than the declined proposal")
                }
            },
            update: { session, update in
                var request = Humata_V1_UpdateSession()
                request.sessionID = session
                if let name = update.name { request.name = name }
                if let archived = update.archived { request.archived = archived }
                if let keeper = update.keeper { request.keeper = keeper }
                if let experimental = update.experimental { request.experimental = experimental }
                if let effort = update.effort { request.effort = effort }
                if let model = update.model { request.model = model }
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
            delete: { session in
                var request = Humata_V1_DeleteSession()
                request.sessionID = session
                guard case .sessionDeleted = try await daemon.call(.deleteSession(request)) else {
                    throw DaemonError.unexpected("something other than the deleted session")
                }
            },
            markSeen: { session, turns in
                var mark = Humata_V1_MarkSeen()
                mark.sessionID = session
                mark.turns = UInt32(clamping: turns)
                _ = try await daemon.call(.markSeen(mark))
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
                return FileContents(contents)
            },
            fileVersions: { session, path in
                var list = Humata_V1_ListFileVersions()
                list.sessionID = session
                list.path = path
                guard case .fileVersions(let versions) = try await daemon.call(.listFileVersions(list)) else {
                    throw DaemonError.unexpected("something other than the file's versions")
                }
                return versions.versions.map {
                    FileVersion(turn: Int($0.turn), time: Date(timeIntervalSince1970: Double($0.timeUnixMs) / 1000))
                }
            },
            readFileVersion: { session, path, turn in
                var read = Humata_V1_ReadFile()
                read.sessionID = session
                read.path = path
                read.turn = UInt32(clamping: turn)
                guard case .fileContents(let contents) = try await daemon.call(.readFile(read)) else {
                    throw DaemonError.unexpected("something other than the file's contents")
                }
                return FileContents(contents)
            },
            restoreFileVersion: { session, path, turn in
                var restore = Humata_V1_RestoreFileVersion()
                restore.sessionID = session
                restore.path = path
                restore.turn = UInt32(clamping: turn)
                guard case .fileVersionRestored(let restored) = try await daemon.call(.restoreFileVersion(restore))
                else {
                    throw DaemonError.unexpected("something other than the restored file")
                }
                return FileContents(restored.contents)
            },
            listFolder: { place, directory in
                var list = Humata_V1_ListFolder()
                list.host = place.host ?? ""
                list.sessionID = place.sessionID ?? ""
                list.root = place.folder ?? ""
                list.directory = directory
                guard case .folder(let folder) = try await daemon.call(.listFolder(list)) else {
                    throw DaemonError.unexpected("something other than the folder's files")
                }
                return Folder(folder)
            },
            readFileAt: { place, path in
                var read = Humata_V1_ReadFile()
                read.host = place.host ?? ""
                read.sessionID = place.sessionID ?? ""
                read.root = place.folder ?? ""
                read.path = path
                guard case .fileContents(let contents) = try await daemon.call(.readFile(read)) else {
                    throw DaemonError.unexpected("something other than the file's contents")
                }
                return FileContents(contents)
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
                create.surface = .app
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
                request.surface = .app
                guard case .itemUpdated(let updated) = try await daemon.call(.updateItem(request)) else {
                    throw DaemonError.unexpected("something other than the updated item")
                }
                return Item(updated.item)
            },
            deleteItem: { item in
                var request = Humata_V1_DeleteItem()
                request.itemID = item
                request.surface = .app
                guard case .itemDeleted = try await daemon.call(.deleteItem(request)) else {
                    throw DaemonError.unexpected("something other than the deleted item")
                }
            },
            boardLog: { project, from, limit in
                var read = Humata_V1_ReadBoardLog()
                read.projectID = project
                read.sinceChangeID = from ?? 0
                read.limit = UInt32(clamping: limit ?? 0)
                guard case .boardLog(let log) = try await daemon.call(.readBoardLog(read)) else {
                    throw DaemonError.unexpected("something other than the board's changes")
                }
                return log.changes.map(BoardChange.init)
            },
            itemLog: { project, item in
                var read = Humata_V1_ReadBoardLog()
                read.projectID = project
                read.itemID = item
                guard case .boardLog(let log) = try await daemon.call(.readBoardLog(read)) else {
                    throw DaemonError.unexpected("something other than the board's changes")
                }
                return log.changes.map(BoardChange.init)
            },
            revert: { change in
                var revert = Humata_V1_RevertBoardChange()
                revert.changeID = change
                revert.surface = .app
                guard case .boardChangeReverted(let reverted) = try await daemon.call(.revertBoardChange(revert))
                else {
                    throw DaemonError.unexpected("something other than the reverted item")
                }
                return Reverted(
                    item: reverted.hasItem ? Item(reverted.item) : nil, kept: reverted.kept.compactMap(ItemField.init))
            },
            inbox: {
                guard case .inbox(let inbox) = try await daemon.call(.listInbox(Humata_V1_ListInbox())) else {
                    throw DaemonError.unexpected("something other than the inbox")
                }
                return Inbox(
                    entries: try inbox.entries.compactMap(InboxEntry.init),
                    approvals: inbox.approvals.compactMap(AdminApproval.init))
            },
            approvals: {
                guard case .approvals(let approvals) = try await daemon.call(.listApprovals(Humata_V1_ListApprovals()))
                else {
                    throw DaemonError.unexpected("something other than the approvals")
                }
                return Approvals(
                    approvals: approvals.approvals.compactMap(AdminApproval.init),
                    grants: approvals.grants.map(ApprovalGrant.init))
            },
            decideApprovals: { ids, approve, scopes, seconds in
                var decide = Humata_V1_DecideApprovals()
                decide.ids = ids
                decide.approve = approve
                decide.grantScopes = scopes.map(\.proto)
                decide.grantSeconds = seconds
                guard case .approvalsDecided(let decided) = try await daemon.call(.decideApprovals(decide)) else {
                    throw DaemonError.unexpected("something other than the decided approvals")
                }
                return decided.approvals.compactMap(AdminApproval.init)
            },
            revokeGrant: { grant in
                var revoke = Humata_V1_RevokeGrant()
                revoke.id = grant
                guard case .grantRevoked = try await daemon.call(.revokeGrant(revoke)) else {
                    throw DaemonError.unexpected("something other than the revoked grant")
                }
            },
            storage: {
                guard case .storage(let storage) = try await daemon.call(.getStorage(Humata_V1_GetStorage())) else {
                    throw DaemonError.unexpected("something other than the storage")
                }
                return Storage(storage)
            },
            setStorageSettings: { autoDelete, days in
                var set = Humata_V1_SetStorageSettings()
                set.settings.autoDelete = autoDelete
                set.settings.days = UInt32(clamping: days)
                guard case .storage(let storage) = try await daemon.call(.setStorageSettings(set)) else {
                    throw DaemonError.unexpected("something other than the storage")
                }
                return Storage(storage)
            },
            cleanUpStorage: { cleanup in
                var clean = Humata_V1_CleanUpStorage()
                clean.deleteArchivedAfterDays = UInt32(clamping: cleanup.deleteAfterDays)
                clean.compact = cleanup.compact
                clean.putAwayWorktrees = cleanup.putAwayWorktrees
                clean.removeUnusedClones = cleanup.removeUnusedClones
                guard case .storage(let storage) = try await daemon.call(.cleanUpStorage(clean)) else {
                    throw DaemonError.unexpected("something other than the storage")
                }
                return Storage(storage)
            },
            skills: {
                guard case .skillList(let list) = try await daemon.call(.listSkills(Humata_V1_ListSkills())) else {
                    throw DaemonError.unexpected("something other than the skills")
                }
                return SkillList(list)
            },
            pickSkill: { name, candidates, winner in
                var pick = Humata_V1_PickSkill()
                pick.name = name
                pick.candidates = candidates
                pick.winner = winner
                guard case .skillPicked = try await daemon.call(.pickSkill(pick)) else {
                    throw DaemonError.unexpected("something other than the skill picked")
                }
            },
            plugins: {
                guard case .pluginList(let list) = try await daemon.call(.listPlugins(Humata_V1_ListPlugins())) else {
                    throw DaemonError.unexpected("something other than the plugins")
                }
                return PluginList(plugins: list.plugins.compactMap(Plugin.init), unreachable: list.unreachable)
            },
            setProjectPlugin: { project, agent, id, enabled in
                var set = Humata_V1_SetProjectPlugin()
                set.projectID = project
                set.agent = agent.proto
                set.id = id
                if let enabled { set.enabled = enabled }
                guard case .projectUpdated(let updated) = try await daemon.call(.setProjectPlugin(set)) else {
                    throw DaemonError.unexpected("something other than the project")
                }
                return Project(updated.project)
            },
            sessionDefaults: {
                let request = Humata_V1_GetSessionDefaults()
                guard case .sessionDefaults(let defaults) = try await daemon.call(.getSessionDefaults(request))
                else {
                    throw DaemonError.unexpected("something other than the session defaults")
                }
                return SessionDefaults(defaults)
            },
            setSessionDefaults: { project, defaults in
                var set = Humata_V1_SetSessionDefaults()
                set.projectID = project ?? ""
                if let defaults { set.defaults = defaults.proto }
                switch try await daemon.call(.setSessionDefaults(set)) {
                case .projectUpdated(let updated): return Project(updated.project)
                case .sessionDefaults: return nil
                default: throw DaemonError.unexpected("something other than the session defaults")
                }
            },
            answerRequest: { request, answer in
                var asked = Humata_V1_AnswerRequest()
                asked.requestID = request
                switch answer {
                case .reply(let text): asked.reply = text
                case .approve: asked.approve = true
                case .move(let status): asked.moveTo = status.proto
                case .dismiss: asked.dismiss = true
                }
                guard case .requestAnswered = try await daemon.call(.answerRequest(asked)) else {
                    throw DaemonError.unexpected("something other than the answered request")
                }
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
            coordinator: {
                guard case .hosts(let hosts) = try await daemon.call(.listHosts(Humata_V1_ListHosts())) else {
                    throw DaemonError.unexpected("something other than the hosts")
                }
                return RemoteHost(hosts.coordinator)
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
            linkCommands: { check in
                var link = Humata_V1_LinkCommands()
                link.check = check
                guard case .commandsLinked(let linked) = try await daemon.call(.linkCommands(link)) else {
                    throw DaemonError.unexpected("something other than the commands' links")
                }
                return CommandLinks(linked)
            },
            setModelServer: { port in
                var set = Humata_V1_SetModelServer()
                set.port = UInt32(port)
                guard case .modelServerSet = try await daemon.call(.setModelServer(set)) else {
                    throw DaemonError.unexpected("something other than the model server being set")
                }
            },
            sshHosts: {
                guard case .sshHosts(let hosts) = try await daemon.call(.listSshHosts(Humata_V1_ListSshHosts())) else {
                    throw DaemonError.unexpected("something other than the SSH config's hosts")
                }
                return hosts.names
            },
            recoveries: {
                guard case .recoveries(let found) = try await daemon.call(.listRecoveries(Humata_V1_ListRecoveries()))
                else {
                    throw DaemonError.unexpected("something other than the recovery copies")
                }
                return found.recoveries.map(Recovery.init)
            },
            restoreSession: { session, host in
                var restore = Humata_V1_RestoreSession()
                restore.sessionID = session
                restore.host = host ?? ""
                guard case .sessionRestored(let restored) = try await daemon.call(.restoreSession(restore)) else {
                    throw DaemonError.unexpected("something other than the restored session")
                }
                return try Session(restored.session)
            },
            restoreHost: { host in
                var restore = Humata_V1_RestoreHost()
                restore.host = host
                guard case .hostRestored(let restored) = try await daemon.call(.restoreHost(restore)) else {
                    throw DaemonError.unexpected("something other than the restored sessions")
                }
                return HostRestore(
                    restored: try restored.restored.map(Session.init),
                    failed: restored.failed.map { RestoreFailure(id: $0.sessionID, message: $0.message) })
            },
            openTerminal: { place, size in
                var open = Humata_V1_OpenTerminal()
                open.host = place.host ?? ""
                open.sessionID = place.sessionID ?? ""
                open.folder = place.folder ?? ""
                open.rows = UInt32(clamping: size.rows)
                open.columns = UInt32(clamping: size.columns)
                guard case .terminalOpened(let opened) = try await daemon.call(.openTerminal(open)) else {
                    throw DaemonError.unexpected("something other than the terminal opened")
                }
                return Terminal(opened.terminal)
            },
            terminals: { host in
                var list = Humata_V1_ListTerminals()
                list.host = host ?? ""
                guard case .terminals(let found) = try await daemon.call(.listTerminals(list)) else {
                    throw DaemonError.unexpected("something other than the terminals")
                }
                return found.terminals.map(Terminal.init)
            },
            typeInTerminal: { terminal, data in
                var input = Humata_V1_TerminalInput()
                input.host = terminal.host ?? ""
                input.terminalID = terminal.id
                input.data = data
                guard case .terminalInputSent = try await daemon.call(.terminalInput(input)) else {
                    throw DaemonError.unexpected("something other than the keys typed")
                }
            },
            resizeTerminal: { terminal, size in
                var resize = Humata_V1_ResizeTerminal()
                resize.host = terminal.host ?? ""
                resize.terminalID = terminal.id
                resize.rows = UInt32(clamping: size.rows)
                resize.columns = UInt32(clamping: size.columns)
                guard case .terminalResized = try await daemon.call(.resizeTerminal(resize)) else {
                    throw DaemonError.unexpected("something other than the terminal resized")
                }
            },
            closeTerminal: { terminal in
                var close = Humata_V1_CloseTerminal()
                close.host = terminal.host ?? ""
                close.terminalID = terminal.id
                guard case .terminalClosed = try await daemon.call(.closeTerminal(close)) else {
                    throw DaemonError.unexpected("something other than the terminal closed")
                }
            },
            attachTerminal: { terminal in
                AsyncThrowingStream { continuation in
                    let task = Task {
                        do {
                            var attach = Humata_V1_AttachTerminal()
                            attach.host = terminal.host ?? ""
                            attach.terminalID = terminal.id
                            for try await item in try await daemon.stream(.attachTerminal(attach)) {
                                guard case .terminalOutput(let output) = item.kind else { continue }
                                continuation.yield(
                                    TerminalOutput(
                                        data: output.data, replay: output.replay,
                                        exitCode: output.exited ? Int(output.exitCode) : nil))
                            }
                            continuation.finish()
                        } catch {
                            continuation.finish(throwing: error)
                        }
                    }
                    continuation.onTermination = { _ in task.cancel() }
                }
            },
            forwardPort: { host, port, session in
                var forward = Humata_V1_ForwardPort()
                forward.host = host ?? ""
                forward.port = UInt32(clamping: port)
                forward.sessionID = session ?? ""
                guard case .portForwarded(let forwarded) = try await daemon.call(.forwardPort(forward)) else {
                    throw DaemonError.unexpected("something other than the port forwarded")
                }
                return Forward(forwarded.forward)
            },
            forwards: {
                guard case .forwards(let forwards) = try await daemon.call(.listForwards(Humata_V1_ListForwards()))
                else {
                    throw DaemonError.unexpected("something other than the forwards")
                }
                return forwards.forwards.map(Forward.init)
            },
            stopForward: { id in
                var stop = Humata_V1_StopForward()
                stop.id = id
                guard case .forwardStopped = try await daemon.call(.stopForward(stop)) else {
                    throw DaemonError.unexpected("something other than the forward stopped")
                }
            },
            search: { query in
                guard case .searchResults(let results) = try await daemon.call(.search(query.proto)) else {
                    throw DaemonError.unexpected("something other than search results")
                }
                return SearchResults(results)
            },
            land: { session, request in
                var land = Humata_V1_LandSession()
                land.sessionID = session
                land.branch = request.branch
                land.message = request.message
                land.push = request.push
                land.openPullRequests = request.openPullRequests
                guard case .sessionLanded(let landed) = try await daemon.call(.landSession(land)) else {
                    throw DaemonError.unexpected("something other than the landed branches")
                }
                return Landed(landed)
            },
            pullRequests: { of, refresh in
                var list = Humata_V1_ListPullRequests()
                switch of {
                case .session(let session): list.sessionID = session
                case .item(let item): list.itemID = item
                }
                list.refresh = refresh
                guard case .pullRequests(let pulls) = try await daemon.call(.listPullRequests(list)) else {
                    throw DaemonError.unexpected("something other than the pull requests")
                }
                return pulls.pullRequests.map(PullRequest.init)
            },
            pullRefresh: {
                let request = Humata_V1_GetPullRefresh()
                guard case .pullRefresh(let refresh) = try await daemon.call(.getPullRefresh(request)) else {
                    throw DaemonError.unexpected("something other than how pull requests are read")
                }
                return PullRefresh(refresh)
            },
            setPullRefresh: { refresh in
                var set = Humata_V1_SetPullRefresh()
                set.settings = refresh.proto
                guard case .pullRefresh(let kept) = try await daemon.call(.setPullRefresh(set)) else {
                    throw DaemonError.unexpected("something other than how pull requests are read")
                }
                return PullRefresh(kept)
            },
            agentAccess: {
                let request = Humata_V1_GetAgentAccess()
                guard case .agentAccess(let access) = try await daemon.call(.getAgentAccess(request)) else {
                    throw DaemonError.unexpected("something other than what agents may do")
                }
                return AgentAccess(access)
            },
            setAgentAccess: { access in
                var set = Humata_V1_SetAgentAccess()
                set.access = access.proto
                guard case .agentAccess(let kept) = try await daemon.call(.setAgentAccess(set)) else {
                    throw DaemonError.unexpected("something other than what agents may do")
                }
                return AgentAccess(kept)
            },
            sessionModels: { session in
                var list = Humata_V1_ListModels()
                list.host = session.host ?? ""
                // For a host that doesn't know `sessionID` yet.
                list.backend = session.backend.proto
                list.sessionID = session.id
                guard case .models(let found) = try await daemon.call(.listModels(list)) else {
                    throw DaemonError.unexpected("something other than the models")
                }
                return found.models.map {
                    ModelChoice(
                        id: $0.id, name: $0.name.isEmpty ? $0.id : $0.name, detail: $0.detail.isEmpty ? nil : $0.detail,
                        readsImages: $0.readsImages, efforts: $0.efforts,
                        defaultEffort: $0.defaultEffort.isEmpty ? nil : $0.defaultEffort)
                }
            },
            savedRepositories: {
                let request = Humata_V1_ListSavedRepositories()
                guard case .savedRepositories(let saved) = try await daemon.call(.listSavedRepositories(request))
                else {
                    throw DaemonError.unexpected("something other than the saved repositories")
                }
                return saved.repositories.map(SavedRepository.init)
            },
            renameSavedRepository: { id, name in
                var request = Humata_V1_RenameSavedRepository()
                request.id = id
                request.name = name
                guard case .savedRepositories(let saved) = try await daemon.call(.renameSavedRepository(request))
                else {
                    throw DaemonError.unexpected("something other than the saved repositories")
                }
                return saved.repositories.map(SavedRepository.init)
            },
            removeSavedRepository: { id in
                var request = Humata_V1_RemoveSavedRepository()
                request.id = id
                guard case .savedRepositories(let saved) = try await daemon.call(.removeSavedRepository(request))
                else {
                    throw DaemonError.unexpected("something other than the saved repositories")
                }
                return saved.repositories.map(SavedRepository.init)
            },
            repositoryOwners: {
                let request = Humata_V1_ListRepositoryOwners()
                guard case .repositoryOwners(let owners) = try await daemon.call(.listRepositoryOwners(request))
                else {
                    throw DaemonError.unexpected("something other than the GitHub owners")
                }
                return owners.owners.map(RepositoryOwner.init)
            },
            addRepositoryOwner: { login in
                var request = Humata_V1_AddRepositoryOwner()
                request.login = login
                guard case .repositoryOwners(let owners) = try await daemon.call(.addRepositoryOwner(request))
                else {
                    throw DaemonError.unexpected("something other than the GitHub owners")
                }
                return owners.owners.map(RepositoryOwner.init)
            },
            removeRepositoryOwner: { login in
                var request = Humata_V1_RemoveRepositoryOwner()
                request.login = login
                guard case .repositoryOwners(let owners) = try await daemon.call(.removeRepositoryOwner(request))
                else {
                    throw DaemonError.unexpected("something other than the GitHub owners")
                }
                return owners.owners.map(RepositoryOwner.init)
            },
            refreshRepositoryOwners: { login in
                var request = Humata_V1_RefreshRepositoryOwners()
                request.login = login ?? ""
                guard
                    case .repositoryOwners(let owners) = try await daemon.call(.refreshRepositoryOwners(request))
                else {
                    throw DaemonError.unexpected("something other than the GitHub owners")
                }
                return owners.owners.map(RepositoryOwner.init)
            },
            updateFromBase: { session in
                var update = Humata_V1_UpdateFromBase()
                update.sessionID = session
                guard case .baseUpdated(let updated) = try await daemon.call(.updateFromBase(update)) else {
                    throw DaemonError.unexpected("something other than the update")
                }
                return BaseUpdate(updated)
            },
            importIssues: { project in
                var request = Humata_V1_ImportIssues()
                request.projectID = project
                guard case .issuesImported(let imported) = try await daemon.call(.importIssues(request)) else {
                    throw DaemonError.unexpected("something other than the imported issues")
                }
                return ImportedIssues(items: imported.items.map(Item.init), skipped: Int(imported.skipped))
            },
            watch: { session, after in
                AsyncThrowingStream { continuation in
                    let task = Task {
                        do {
                            var watch = Humata_V1_WatchSession()
                            watch.sessionID = session
                            watch.afterEventID = after ?? ""
                            for try await item in try await daemon.stream(.watchSession(watch)) {
                                switch item.kind {
                                case .sessionEvent(let event):
                                    if let event = SessionEvent(event) { continuation.yield(.event(event)) }
                                case .historyLoading(let loading):
                                    continuation.yield(.loading(read: loading.read, total: loading.total))
                                case .historyLoaded:
                                    continuation.yield(.historyLoaded)
                                default: break
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
    /// The connection being opened, which calls made meanwhile wait for
    /// instead of each opening their own.
    private var opening: Task<Connection, any Error>?

    init(socket: String, programVersion: String) {
        self.socket = socket
        self.programVersion = programVersion
    }

    private func connected() async throws -> Connection {
        if let connection, !connection.isClosed { return connection }
        if let opening { return try await opening.value }
        let (socket, programVersion) = (socket, programVersion)
        let task = Task { try await Connection.open(path: socket, programVersion: programVersion) }
        opening = task
        // Cleared whether it opened or not, so the next call tries again.
        defer { opening = nil }
        let opened = try await task.value
        connection = opened
        return opened
    }

    func call(_ request: Humata_V1_Request.OneOf_Kind) async throws -> Humata_V1_Response.OneOf_Kind {
        let response = try await connected().call(request)
        // Only another client's actions wait for approval; the app's run.
        if case .needsApproval = response { throw DaemonError.notTakenForTheApp }
        return response
    }

    func stream(_ request: Humata_V1_Request.OneOf_Kind) async throws -> Items {
        try await connected().stream(request)
    }
}
