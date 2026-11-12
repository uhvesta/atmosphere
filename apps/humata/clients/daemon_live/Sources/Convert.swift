import Foundation
import HumataDaemonClient
import HumataProto

extension ErrorCode {
    init(_ code: Humata_V1_ErrorCode) {
        self =
            switch code {
            case .unknownSession: .unknownSession
            case .unknownEvent: .unknownEvent
            case .unknownPermissionRequest: .unknownPermissionRequest
            case .unknownBackend: .unknownBackend
            case .busy: .busy
            case .invalidRequest: .invalidRequest
            case .backendFailed: .backendFailed
            case .storageFailed: .storageFailed
            case .hostUnavailable: .hostUnavailable
            case .unknownProject: .unknownProject
            case .archived: .archived
            case .unknownItem: .unknownItem
            case .unknownBoardChange: .unknownBoardChange
            case .unknownHost: .unknownHost
            case .unknownThread: .unknownThread
            case .changed: .changed
            case .unknownQueuedMessage: .unknownQueuedMessage
            case .unknownImport: .unknownImport
            case .alreadyImported: .alreadyImported
            case .unknownRequest: .unknownRequest
            case .unspecified, .UNRECOGNIZED: .unknown
            }
    }
}

extension Backend {
    init?(_ backend: Humata_V1_Backend) {
        switch backend {
        case .scripted: self = .scripted
        case .local: self = .local
        case .copilot: self = .copilot
        case .claude: self = .claude
        case .unspecified, .UNRECOGNIZED: return nil
        }
    }

    var proto: Humata_V1_Backend {
        switch self {
        case .scripted: .scripted
        case .local: .local
        case .copilot: .copilot
        case .claude: .claude
        }
    }
}

extension ApprovalMode {
    init(_ mode: Humata_V1_ApprovalMode) {
        self =
            switch mode {
            case .allowEdits: .allowEdits
            case .allowEverything: .allowEverything
            case .askEveryTime, .unspecified, .UNRECOGNIZED: .askEveryTime
            }
    }

    var proto: Humata_V1_ApprovalMode {
        switch self {
        case .askEveryTime: .askEveryTime
        case .allowEdits: .allowEdits
        case .allowEverything: .allowEverything
        }
    }
}

extension Repository {
    init(_ repository: Humata_V1_Repository) {
        self.init(name: repository.name, path: repository.path)
    }

    var proto: Humata_V1_Repository {
        var repository = Humata_V1_Repository()
        repository.name = name
        repository.path = path
        return repository
    }
}

extension Project {
    init(_ project: Humata_V1_Project) {
        self.init(
            id: project.id, name: project.name, repositories: project.repositories.map(Repository.init),
            instructions: project.instructions,
            created: Date(timeIntervalSince1970: Double(project.createdUnixMs) / 1000))
    }
}

private func optional(_ text: String) -> String? {
    text.isEmpty ? nil : text
}

extension Session {
    init(_ info: Humata_V1_SessionInfo) throws {
        guard let backend = Backend(info.backend) else {
            throw DaemonError.unexpected("a session on a backend this app doesn't know")
        }
        let state: SessionState =
            switch info.state {
            case .working: .working
            case .waitingForApproval: .waitingForApproval
            case .failed: .failed
            case .idle, .unspecified, .UNRECOGNIZED: .idle
            }
        self.init(
            id: info.id, name: info.name, backend: backend, workingDirectory: info.workingDirectory,
            model: optional(info.model), state: state, parentID: optional(info.parentID),
            forkedBeforeEventID: optional(info.forkedBeforeEventID), host: optional(info.host),
            approvalMode: ApprovalMode(info.approvalMode), projectID: optional(info.projectID),
            archived: info.archived, keeper: info.keeper, itemID: optional(info.itemID), turns: Int(info.turns),
            seenTurns: Int(info.seenTurns),
            pending: info.pending.map {
                PendingPermission(requestID: $0.requestID, kind: PermissionKind($0.kind), summary: $0.summary)
            },
            hostOffline: info.hostOffline,
            lastActive: info.lastActiveUnixMs == 0 ? nil : date(info.lastActiveUnixMs))
    }
}

private func date(_ unixMs: Int64) -> Date {
    Date(timeIntervalSince1970: Double(unixMs) / 1000)
}

extension ItemStatus {
    init(_ status: Humata_V1_ItemStatus) {
        self =
            switch status {
            case .inProgress: .inProgress
            case .waitingOnYou: .waitingOnYou
            case .inReview: .inReview
            case .done: .done
            case .wontDo: .wontDo
            case .todo, .unspecified, .UNRECOGNIZED: .todo
            }
    }

    var proto: Humata_V1_ItemStatus {
        switch self {
        case .todo: .todo
        case .inProgress: .inProgress
        case .waitingOnYou: .waitingOnYou
        case .inReview: .inReview
        case .done: .done
        case .wontDo: .wontDo
        }
    }
}

extension Priority {
    init(_ priority: Humata_V1_Priority) {
        self =
            switch priority {
            case .low: .low
            case .medium: .medium
            case .high: .high
            case .urgent: .urgent
            case .none, .UNRECOGNIZED: .none
            }
    }

    var proto: Humata_V1_Priority {
        switch self {
        case .none: .none
        case .low: .low
        case .medium: .medium
        case .high: .high
        case .urgent: .urgent
        }
    }
}

extension ItemFields {
    init(_ fields: Humata_V1_ItemFields) {
        self.init(
            parentID: optional(fields.parentID), title: fields.title, spec: fields.spec,
            status: ItemStatus(fields.status), priority: Priority(fields.priority))
    }
}

extension Item {
    init(_ item: Humata_V1_Item) {
        self.init(
            id: item.id, projectID: item.projectID,
            fields: ItemFields(
                parentID: optional(item.parentID), title: item.title, spec: item.spec,
                status: ItemStatus(item.givenStatus), priority: Priority(item.priority)),
            status: ItemStatus(item.status), created: date(item.createdUnixMs), updated: date(item.updatedUnixMs),
            sessionIDs: item.sessionIds)
    }
}

extension BoardChange {
    init(_ change: Humata_V1_BoardChange) {
        self.init(
            id: change.id, itemID: change.itemID, time: date(change.timeUnixMs),
            before: change.hasBefore ? ItemFields(change.before) : nil,
            after: change.hasAfter ? ItemFields(change.after) : nil,
            reverts: change.reverts == 0 ? nil : change.reverts)
    }
}

extension InboxEntry {
    /// Nil for a reason this app doesn't know, from a newer daemon.
    init?(_ entry: Humata_V1_InboxEntry) throws {
        let reason: InboxReason
        switch entry.reason {
        case .approval: reason = .approval
        case .reply: reason = .reply
        case .failed: reason = .failed
        case .question: reason = .question
        case .move: reason = .move
        case .unspecified, .UNRECOGNIZED: return nil
        }
        let request = entry.hasRequest ? AgentRequest(entry.request) : nil
        if [.question, .move].contains(reason), request == nil { return nil }
        self.init(
            session: try Session(entry.session), reason: reason, projectName: optional(entry.projectName),
            itemTitle: optional(entry.itemTitle), request: request)
    }
}

extension AgentRequest {
    /// Nil for a kind this app doesn't know, from a newer daemon.
    init?(_ request: Humata_V1_AgentRequest) {
        let kind: Kind
        switch request.kind {
        case .question: kind = .question
        case .move: kind = .move(item: request.itemID, status: ItemStatus(request.status))
        case .unspecified, .UNRECOGNIZED: return nil
        }
        self.init(
            id: request.id, sessionID: request.sessionID, kind: kind, text: request.text,
            created: date(request.createdUnixMs))
    }
}

extension SessionEvent {
    /// Nil for a kind of event this app doesn't know, from a newer daemon.
    init?(_ event: Humata_V1_SessionEvent) {
        let kind: Kind
        switch event.kind {
        case .userMessage(let message): kind = .userMessage(message.text)
        case .turnStarted: kind = .turnStarted
        case .assistantDelta(let delta): kind = .assistantDelta(delta.text)
        case .assistantMessage(let message): kind = .assistantMessage(message.text)
        case .reasoning(let reasoning): kind = .reasoning(reasoning.text)
        case .toolStarted(let tool):
            kind = .toolStarted(callID: tool.callID, tool: tool.tool, argumentsJSON: tool.argumentsJson)
        case .toolCompleted(let tool):
            kind = .toolCompleted(callID: tool.callID, success: tool.success, output: tool.output)
        case .permissionRequested(let request):
            kind = .permissionRequested(
                requestID: request.requestID, kind: PermissionKind(request.kind), summary: request.summary)
        case .permissionCompleted(let completed):
            kind = .permissionCompleted(
                requestID: completed.requestID, approved: completed.approved, automatic: completed.automatic)
        case .turnEnded(let ended): kind = .turnEnded(aborted: ended.aborted)
        case .idle: kind = .idle
        case .failed(let failed): kind = .failed(failed.message)
        case .usage(let usage):
            kind = .usage(
                model: usage.model, inputTokens: usage.inputTokens, outputTokens: usage.outputTokens,
                premiumRequests: usage.premiumRequests)
        case .checkpoint: kind = .checkpoint
        case .messageQueued(let queued): kind = .messageQueued(queueID: queued.queueID, text: queued.text)
        case .dequeued(let dequeued): kind = .dequeued(queueID: dequeued.queueID)
        case nil: return nil
        }
        self.init(id: event.id, time: Date(timeIntervalSince1970: Double(event.timeUnixMs) / 1000), kind: kind)
    }
}

extension RemoteHost {
    init(_ host: Humata_V1_HostInfo) {
        self.init(
            name: host.name, sshDestination: optional(host.sshDestination), envNames: host.envNames,
            setupScript: host.setupScript)
    }
}

extension SetupResult {
    /// Nil when there was no script to run.
    init?(_ setup: Humata_V1_SetupResult) {
        guard setup.ran else { return nil }
        self.init(succeeded: setup.succeeded, output: setup.output, time: date(setup.ranUnixMs))
    }
}

extension HostHealth {
    init(_ health: Humata_V1_HostHealth) {
        guard health.reachable else {
            self = .unreachable(health.problem)
            return
        }
        let status = health.status
        self = .reachable(
            HostStatus(
                programVersion: status.programVersion, platform: status.platform,
                freeDiskBytes: status.freeDiskBytes,
                copilot: status.copilotVersion.isEmpty
                    ? .failure(ProgramProblem(status.copilotProblem)) : .success(status.copilotVersion),
                githubToken: status.githubToken, setup: status.hasSetup ? SetupResult(status.setup) : nil,
                claude: status.claudeVersion.isEmpty
                    ? .failure(ProgramProblem(status.claudeProblem)) : .success(status.claudeVersion),
                anthropicKey: status.anthropicKey))
    }
}

extension ChangesQuery {
    func proto(_ session: String) -> Humata_V1_ReadChanges {
        var read = Humata_V1_ReadChanges()
        read.sessionID = session
        switch range {
        case .session: read.range.kind = .session
        case .unstaged: read.range.kind = .unstaged
        case .staged: read.range.kind = .staged
        case .branch(let branch):
            read.range.kind = .branch
            read.range.branch = branch
        case .turn(let turn):
            read.range.kind = .turn
            read.range.turn = UInt32(max(turn, 0))
        }
        read.context = UInt32(max(context, 0))
        if let file { read.file = file.proto }
        read.wholeFile = wholeFile
        return read
    }
}

extension FileRef {
    init(_ file: Humata_V1_FileRef) {
        self.init(repository: file.repository, path: file.path)
    }

    var proto: Humata_V1_FileRef {
        var file = Humata_V1_FileRef()
        file.repository = repository
        file.path = path
        return file
    }
}

extension ChangeAction {
    var proto: Humata_V1_ChangeAction {
        switch self {
        case .stage: .stage
        case .unstage: .unstage
        case .revert: .revert
        }
    }
}

extension RepositoryChanges {
    init(_ changes: Humata_V1_RepositoryChanges) {
        self.init(name: changes.name, patch: changes.patch, files: changes.files.map(FileDiff.init))
    }
}

extension FileDiff {
    init(_ file: Humata_V1_FileDiff) {
        self.init(
            path: file.path,
            oldPath: file.oldPath.isEmpty ? nil : file.oldPath,
            status: FileStatus(file.status),
            binary: file.binary,
            blob: file.blob,
            hunks: file.hunks.map(Hunk.init),
            viewed: file.viewed
        )
    }
}

extension FileStatus {
    init(_ status: Humata_V1_FileStatus) {
        self =
            switch status {
            case .added: .added
            case .deleted: .deleted
            case .renamed: .renamed
            case .unchanged: .unchanged
            case .modified, .UNRECOGNIZED: .modified
            }
    }
}

extension Hunk {
    init(_ hunk: Humata_V1_Hunk) {
        self.init(
            oldStart: Int(hunk.oldStart),
            oldCount: Int(hunk.oldCount),
            newStart: Int(hunk.newStart),
            newCount: Int(hunk.newCount),
            header: hunk.header,
            lines: hunk.lines.map(DiffLine.init)
        )
    }
}

extension DiffLine {
    init(_ line: Humata_V1_DiffLine) {
        let kind: Kind =
            switch line.kind {
            case .added: .added
            case .removed: .removed
            case .context, .UNRECOGNIZED: .context
            }
        self.init(
            kind: kind,
            old: line.old == 0 ? nil : Int(line.old),
            new: line.new == 0 ? nil : Int(line.new),
            text: line.text
        )
    }
}

extension ReviewThread {
    init(_ thread: Humata_V1_ReviewThread) {
        self.init(
            id: thread.id,
            file: FileRef(thread.file),
            startLine: Int(thread.startLine),
            endLine: Int(thread.endLine),
            outdated: thread.outdated,
            code: thread.code,
            quote: thread.quote,
            resolved: thread.resolved,
            comments: thread.comments.map(ReviewComment.init)
        )
    }

    init(saved response: Humata_V1_Response.OneOf_Kind) throws {
        guard case .threadSaved(let saved) = response else {
            throw DaemonError.unexpected("something other than the saved thread")
        }
        self.init(saved.thread)
    }
}

extension ReviewComment {
    init(_ comment: Humata_V1_ReviewComment) {
        self.init(
            id: comment.id,
            author: comment.author == .agent ? .agent : .you,
            text: comment.text,
            sent: comment.sent,
            time: date(comment.timeUnixMs),
            turn: comment.turn == 0 ? nil : Int(comment.turn)
        )
    }
}

extension ChatFile {
    init(_ file: Humata_V1_ChatFile) {
        self.init(
            path: file.path, size: file.size,
            modified: Date(timeIntervalSince1970: TimeInterval(file.modifiedUnixMs) / 1000))
    }
}

extension PermissionKind {
    init(_ kind: Humata_V1_PermissionKind) {
        self =
            switch kind {
            case .shell: .shell
            case .write: .write
            case .read: .read
            case .other, .unspecified, .UNRECOGNIZED: .other
            }
    }
}

extension Recovery {
    init(_ recovery: Humata_V1_Recovery) {
        self.init(
            id: recovery.sessionID, name: recovery.name, host: recovery.host,
            backedUpTurns: Int(recovery.backedUpTurns), turns: Int(recovery.turns),
            backedUp: Date(timeIntervalSince1970: Double(recovery.backedUpUnixMs) / 1000),
            onHost: recovery.onHost)
    }
}

extension SkippedSession {
    init(_ session: Humata_V1_SkippedSession) throws {
        guard let backend = Backend(session.backend) else {
            throw DaemonError.unexpected("a skipped session on a backend this app doesn't know")
        }
        self.init(backend: backend, id: session.id, folder: session.folder, reason: session.reason)
    }
}

extension ImportableSession {
    init(_ session: Humata_V1_ImportableSession) throws {
        guard let backend = Backend(session.backend) else {
            throw DaemonError.unexpected("a session to import on a backend this app doesn't know")
        }
        self.init(
            backend: backend, id: session.id, workingDirectory: session.workingDirectory, summary: session.summary,
            updated: Date(timeIntervalSince1970: Double(session.updatedUnixMs) / 1000))
    }
}
