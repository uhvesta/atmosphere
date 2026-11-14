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
            case .unknownImage: .unknownImage
            case .unknownImport: .unknownImport
            case .alreadyImported: .alreadyImported
            case .unknownRequest: .unknownRequest
            case .unknownTerminal: .unknownTerminal
            case .unknownConnection: .unknownConnection
            case .unknownForward: .unknownForward
            case .unknownProposal: .unknownProposal
            case .github: .github
            case .appOnly: .appOnly
            case .unknownApproval: .unknownApproval
            case .skillClash: .skillClash
            case .accessOff: .accessOff
            case .dependencyCycle: .dependencyCycle
            case .invalidDependency: .invalidDependency
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
            case .auto: .auto
            case .askEveryTime, .unspecified, .UNRECOGNIZED: .askEveryTime
            }
    }

    var proto: Humata_V1_ApprovalMode {
        switch self {
        case .askEveryTime: .askEveryTime
        case .allowEdits: .allowEdits
        case .allowEverything: .allowEverything
        case .auto: .auto
        }
    }
}

extension Repository {
    init(_ repository: Humata_V1_Repository) {
        self.init(name: repository.name, path: repository.path, url: repository.url)
    }

    var proto: Humata_V1_Repository {
        var repository = Humata_V1_Repository()
        repository.name = name
        repository.path = path
        repository.url = url
        return repository
    }
}

extension Project {
    init(_ project: Humata_V1_Project) {
        self.init(
            id: project.id, name: project.name, repositories: project.repositories.map(Repository.init),
            instructions: project.instructions, skills: project.skills.map(Skill.init),
            plugins: project.plugins.compactMap(PluginChoice.init),
            defaults: project.hasDefaults ? SessionDefaults(project.defaults) : nil,
            // An older daemon says nothing of it, and asks for nothing.
            agentSessions: project.hasAgentSessions ? AgentSessions(project.agentSessions) : AgentSessions(),
            created: Date(timeIntervalSince1970: Double(project.createdUnixMs) / 1000))
        planWithCode = project.planWithCode
    }
}

extension Skill {
    init(_ skill: Humata_V1_Skill) {
        self.init(name: skill.name, text: skill.text)
    }

    var proto: Humata_V1_Skill {
        var skill = Humata_V1_Skill()
        skill.name = name
        skill.text = text
        return skill
    }
}

extension Proposal {
    init(_ proposal: Humata_V1_Proposal) {
        let state: Proposal.State =
            switch proposal.state {
            case .applied: .applied(itemID: proposal.appliedItemID)
            case .declined: .declined
            case .pending, .unspecified, .UNRECOGNIZED: .pending
            }
        self.init(
            id: proposal.id,
            kind: proposal.kind == .specEdit ? .specEdit : proposal.kind == .dependency ? .dependency : .task,
            title: proposal.title,
            spec: proposal.spec, parentID: optional(proposal.parentID), itemID: optional(proposal.itemID),
            note: proposal.note, state: state,
            currentSpec: proposal.kind == .specEdit && state == .pending ? proposal.currentSpec : nil,
            itemKind: ItemKind(proposal.itemKind), dependsOn: proposal.dependsOn,
            dependsOnTitles: proposal.dependsOnTitles)
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
            // To the minute, as it's shown, so each tool call doesn't
            // redraw the sidebar.
            lastActive: info.lastActiveUnixMs == 0 ? nil : date(info.lastActiveUnixMs / 60_000 * 60_000),
            planning: info.planning,
            asking: info.asking, lastMessage: optional(info.lastMessage), itemTitle: optional(info.itemTitle),
            effort: optional(info.effort), experimental: info.experimental)
        readOnly = info.readOnly
        sharesCode = info.sharesCode
        startedBy = optional(info.startedBySessionID)
        // A host from before access levels says only whether it's a side chat.
        access = AccessLevel(info.access) ?? (info.readOnly ? .sideChat : .full)
        codeCommits = info.codeCommits.map { CodeCommit(repository: $0.repository, commit: $0.commit) }
    }
}

extension AccessLevel {
    /// Nil for none, or one this app doesn't know.
    init?(_ level: Humata_V1_AccessLevel) {
        switch level {
        case .full: self = .full
        case .plan: self = .plan
        case .sideChat: self = .sideChat
        case .unspecified, .UNRECOGNIZED: return nil
        }
    }

    var proto: Humata_V1_AccessLevel {
        switch self {
        case .full: .full
        case .plan: .plan
        case .sideChat: .sideChat
        }
    }
}

extension AgentSessions {
    init(_ sessions: Humata_V1_AgentSessions) {
        let mode: Mode =
            switch sessions.mode {
            case .off: .off
            case .allow: .allow
            case .ask, .unspecified, .UNRECOGNIZED: .ask
            }
        self.init(
            mode: mode, messageOthers: sessions.messageOthers, maxChildren: Int(sessions.maxChildren),
            maxDepth: Int(sessions.maxDepth))
    }

    var proto: Humata_V1_AgentSessions {
        var sessions = Humata_V1_AgentSessions()
        sessions.mode =
            switch mode {
            case .off: .off
            case .ask: .ask
            case .allow: .allow
            }
        sessions.messageOthers = messageOthers
        sessions.maxChildren = UInt32(clamping: maxChildren)
        sessions.maxDepth = UInt32(clamping: maxDepth)
        return sessions
    }
}

extension SessionDefaults {
    init(_ defaults: Humata_V1_SessionDefaults) {
        self.init(
            backend: Backend(defaults.backend), model: optional(defaults.model), effort: optional(defaults.effort),
            experimental: defaults.experimental)
    }

    var proto: Humata_V1_SessionDefaults {
        var defaults = Humata_V1_SessionDefaults()
        if let backend { defaults.backend = backend.proto }
        defaults.model = model ?? ""
        defaults.effort = effort ?? ""
        defaults.experimental = experimental
        return defaults
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

extension ItemKind {
    /// Nil when it isn't said, or is a kind this app doesn't know.
    init?(_ kind: Humata_V1_ItemKind) {
        switch kind {
        case .milestone: self = .milestone
        case .task: self = .task
        case .unspecified, .UNRECOGNIZED: return nil
        }
    }

    var proto: Humata_V1_ItemKind {
        switch self {
        case .milestone: .milestone
        case .task: .task
        }
    }
}

extension ItemFields {
    init(_ fields: Humata_V1_ItemFields) {
        self.init(
            parentID: optional(fields.parentID), title: fields.title, spec: fields.spec,
            status: ItemStatus(fields.status), priority: Priority(fields.priority), kind: ItemKind(fields.kind),
            dependsOn: fields.dependsOn)
    }
}

extension Item {
    init(_ item: Humata_V1_Item) {
        self.init(
            id: item.id, projectID: item.projectID,
            fields: ItemFields(
                parentID: optional(item.parentID), title: item.title, spec: item.spec,
                status: ItemStatus(item.givenStatus), priority: Priority(item.priority),
                kind: ItemKind(item.kind), dependsOn: item.dependsOn),
            status: ItemStatus(item.status), created: date(item.createdUnixMs), updated: date(item.updatedUnixMs),
            sessionIDs: item.sessionIds, blocked: item.blocked, waitingOn: item.waitingOn,
            planOrder: Int(item.planOrder))
    }
}

extension BoardChange {
    init(_ change: Humata_V1_BoardChange) {
        self.init(
            id: change.id, itemID: change.itemID, time: date(change.timeUnixMs),
            before: change.hasBefore ? ItemFields(change.before) : nil,
            after: change.hasAfter ? ItemFields(change.after) : nil,
            reverts: change.reverts == 0 ? nil : change.reverts, author: BoardAuthor(change),
            approvalID: change.approvalID.isEmpty ? nil : change.approvalID,
            grantID: change.grantID.isEmpty ? nil : change.grantID)
    }
}

extension ApprovalScope {
    init?(_ scope: Humata_V1_ApprovalScope) {
        switch scope {
        case .proposals: self = .proposals
        case .board: self = .board
        case .destructive: self = .destructive
        case .unspecified, .UNRECOGNIZED: return nil
        }
    }

    var proto: Humata_V1_ApprovalScope {
        switch self {
        case .proposals: .proposals
        case .board: .board
        case .destructive: .destructive
        }
    }
}

extension AdminApproval {
    /// Nil for one of a scope or state this app doesn't know, from a newer daemon.
    init?(_ approval: Humata_V1_Approval) {
        guard let scope = ApprovalScope(approval.scope) else { return nil }
        let state: ApprovalState
        switch approval.state {
        case .pending: state = .pending
        case .approved: state = .approved
        case .denied: state = .denied
        case .unspecified, .UNRECOGNIZED: return nil
        }
        var failure: String?
        if approval.hasResult, case .error(let error) = approval.result.kind {
            failure = error.message
        }
        self.init(
            id: approval.id, client: approval.client, parent: approval.parent, scope: scope,
            summary: approval.summary, before: approval.before, after: approval.after, state: state,
            asked: date(approval.askedUnixMs),
            decided: approval.decidedUnixMs == 0 ? nil : date(approval.decidedUnixMs),
            grantID: approval.grantID.isEmpty ? nil : approval.grantID, failure: failure,
            projectID: approval.projectID.isEmpty ? nil : approval.projectID)
    }
}

extension Storage {
    init(_ storage: Humata_V1_Storage) {
        self.init(
            uses: storage.uses.compactMap { use in
                Storage.Kind(use.kind).map { Storage.Use(kind: $0, bytes: use.bytes, path: use.path) }
            },
            autoDelete: storage.settings.autoDelete, days: Int(storage.settings.days),
            job: storage.hasJob
                ? Storage.Job(step: storage.job.step, done: Int(storage.job.done), total: Int(storage.job.total))
                : nil,
            skipped: storage.skipped.map { Storage.Skip(sessionID: $0.sessionID, name: $0.name, reason: $0.reason) },
            deletedSessions: Int(storage.deletedSessions),
            cleaned: storage.cleanedUnixMs == 0 ? nil : date(storage.cleanedUnixMs),
            failure: storage.failure.isEmpty ? nil : storage.failure,
            archivedWorktreesBytes: storage.archivedWorktreesBytes, unusedClonesBytes: storage.unusedClonesBytes)
    }
}

extension Storage.Kind {
    init?(_ kind: Humata_V1_StorageKind) {
        switch kind {
        case .clones: self = .clones
        case .worktrees: self = .worktrees
        case .histories: self = .histories
        case .recovery: self = .recovery
        case .store: self = .store
        case .search: self = .search
        case .logs: self = .logs
        case .unspecified, .UNRECOGNIZED: return nil
        }
    }
}

extension ApprovalGrant {
    init(_ grant: Humata_V1_Grant) {
        self.init(
            id: grant.id, scopes: grant.scopes.compactMap(ApprovalScope.init), granted: date(grant.grantedUnixMs),
            expires: date(grant.expiresUnixMs), used: Int(grant.used))
    }
}

extension BoardAuthor {
    init(_ change: Humata_V1_BoardChange) {
        if !change.sessionID.isEmpty {
            self = .session(change.sessionID)
            return
        }
        switch change.surface {
        case .app: self = .app
        case .commandLine: self = .commandLine
        case .mcp: self = .mcp
        case .humata: self = .humata
        case .unspecified, .UNRECOGNIZED: self = .unknown
        }
    }
}

extension ItemField {
    /// Nil for a field this app doesn't know, from a newer daemon.
    init?(_ field: Humata_V1_ItemField) {
        switch field {
        case .title: self = .title
        case .spec: self = .spec
        case .status: self = .status
        case .priority: self = .priority
        case .kind: self = .kind
        case .dependsOn: self = .dependsOn
        case .unspecified, .UNRECOGNIZED: return nil
        }
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
        case .startSession: kind = .startSession(name: request.name)
        case .sendMessage: kind = .sendMessage(to: request.targetSessionID, name: request.name)
        case .unspecified, .UNRECOGNIZED: return nil
        }
        self.init(
            id: request.id, sessionID: request.sessionID, kind: kind, text: request.text,
            created: date(request.createdUnixMs))
    }
}

extension ChatCommands {
    init(_ commands: Humata_V1_ChatCommands) {
        self.init(
            compact: commands.compact, compactTakesFocus: commands.compactTakesFocus, side: commands.side,
            plan: commands.plan)
    }
}

extension SessionEvent {
    /// Nil for a kind of event this app doesn't know, from a newer daemon.
    init?(_ event: Humata_V1_SessionEvent) {
        let kind: Kind
        switch event.kind {
        case .userMessage(let message): kind = .userMessage(message.text, images: message.images)
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
                requestID: completed.requestID, approved: completed.approved, automatic: completed.automatic,
                judged: completed.judged)
        case .turnEnded(let ended): kind = .turnEnded(aborted: ended.aborted)
        case .idle: kind = .idle
        case .failed(let failed): kind = .failed(failed.message)
        case .waiting(let waiting): kind = .waiting(waiting.reason)
        case .notice(let notice): kind = .notice(notice.text)
        case .rewound(let rewound):
            kind = .rewound(text: rewound.text, images: rewound.images, files: rewound.files)
        case .rewindUndone: kind = .rewindUndone
        case .compacted: kind = .compacted
        case .compacting(let compacting): kind = .compacting(focus: compacting.focus)
        case .usage(let usage):
            kind = .usage(
                model: usage.model, inputTokens: usage.inputTokens, outputTokens: usage.outputTokens,
                premiumRequests: usage.premiumRequests)
        case .checkpoint: kind = .checkpoint
        case .messageQueued(let queued):
            kind = .messageQueued(queueID: queued.queueID, text: queued.text, images: queued.images)
        case .dequeued(let dequeued): kind = .dequeued(queueID: dequeued.queueID, dropped: dequeued.dropped)
        // A save part way through a turn only matters to forks and restores.
        case .saved, nil: return nil
        case .subagent(let subagent):
            let status: Subagent.Status
            switch subagent.status {
            case .running: status = .running
            case .completed: status = .completed
            case .failed: status = .failed
            case .stopped: status = .stopped
            // A status from a newer daemon: it isn't known to be running,
            // so nothing waits on it.
            case .unspecified, .UNRECOGNIZED: status = .stopped
            }
            kind = .subagent(
                Subagent(
                    agentType: subagent.agentType, description: subagent.description_p, status: status,
                    totalTokens: subagent.totalTokens, toolUses: subagent.toolUses,
                    duration: .milliseconds(Int64(clamping: subagent.durationMs))))
        }
        self.init(
            id: event.id, time: Date(timeIntervalSince1970: Double(event.timeUnixMs) / 1000), kind: kind,
            parentCallID: optional(event.parentCallID))
    }
}

extension RemoteHost {
    init(_ host: Humata_V1_HostInfo) {
        self.init(
            name: host.name, sshDestination: optional(host.sshDestination), envNames: host.envNames,
            setupScript: host.setupScript, providers: host.providers.compactMap(HostProvider.init))
    }
}

extension HostProvider {
    /// Nil for a kind this app doesn't know.
    init?(_ provider: Humata_V1_Provider) {
        let kind: Kind
        switch provider.kind {
        case .anthropic: kind = .anthropic
        case .openai: kind = .openAI
        case .opencodeGo: kind = .openCodeGo
        case .unspecified, .UNRECOGNIZED: return nil
        }
        self.init(
            id: provider.id, name: provider.name, kind: kind, baseURL: provider.baseURL,
            keyVariable: provider.keyVariable, wireAPI: WireAPI(rawValue: provider.wireApi) ?? .completions)
    }

    var message: Humata_V1_Provider {
        var provider = Humata_V1_Provider()
        provider.id = id
        provider.name = name
        switch kind {
        case .anthropic: provider.kind = .anthropic
        case .openAI: provider.kind = .openai
        case .openCodeGo: provider.kind = .opencodeGo
        }
        provider.baseURL = baseURL
        provider.keyVariable = keyVariable
        provider.wireApi = takesWireAPI ? wireAPI.rawValue : ""
        return provider
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
                anthropicKey: status.anthropicKey,
                environmentProblem: status.environmentProblem.isEmpty ? nil : status.environmentProblem))
    }
}

extension CommandLinks {
    init(_ linked: Humata_V1_CommandsLinked) {
        self.init(
            folder: linked.folder,
            links: linked.links.map { link in
                let state: Link.State =
                    switch link.state {
                    case .linked: .linked
                    case .stale: .stale
                    case .other: .other
                    case .missing, .unspecified, .UNRECOGNIZED: .missing
                    }
                return Link(name: link.name, state: state)
            },
            onPath: linked.onPath,
            environmentProblem: linked.environmentProblem.isEmpty ? nil : linked.environmentProblem)
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
            author: comment.author == .agent ? .agent : comment.author == .github ? .github(comment.login) : .you,
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
            onHost: recovery.onHost, problem: recovery.problem)
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

extension Terminal {
    init(_ terminal: Humata_V1_Terminal) {
        self.init(
            id: terminal.id, host: terminal.host.isEmpty ? nil : terminal.host,
            sessionID: terminal.sessionID.isEmpty ? nil : terminal.sessionID, folder: terminal.folder,
            shell: terminal.shell, opened: Date(timeIntervalSince1970: Double(terminal.openedUnixMs) / 1000),
            exitCode: terminal.exited ? Int(terminal.exitCode) : nil)
    }
}

extension Forward {
    init(_ forward: Humata_V1_Forward) {
        self.init(
            id: forward.id, host: forward.host.isEmpty ? nil : forward.host, port: Int(forward.port),
            localPort: Int(forward.localPort), sessionID: forward.sessionID.isEmpty ? nil : forward.sessionID)
    }
}

extension FileContents {
    init(_ contents: Humata_V1_FileContents) {
        self.init(
            path: contents.path, text: contents.binary ? nil : contents.text,
            data: contents.data.isEmpty ? nil : contents.data, size: contents.size, truncated: contents.truncated)
    }
}

extension Folder {
    init(_ folder: Humata_V1_Folder) {
        self.init(
            root: folder.root, directory: folder.directory,
            entries: folder.entries.map {
                FolderEntry(
                    name: $0.name, isDirectory: $0.isDirectory, size: $0.size,
                    modified: Date(timeIntervalSince1970: Double($0.modifiedUnixMs) / 1000))
            },
            truncated: folder.truncated)
    }
}

extension SearchQuery {
    var proto: Humata_V1_Search {
        var search = Humata_V1_Search()
        search.query = text
        search.related = related
        search.projectID = projectID ?? ""
        if let since { search.sinceUnixMs = Int64((since.timeIntervalSince1970 * 1000).rounded()) }
        if let until { search.untilUnixMs = Int64((until.timeIntervalSince1970 * 1000).rounded()) }
        return search
    }
}

extension SearchResults {
    init(_ results: Humata_V1_SearchResults) {
        func time(_ unixMs: Int64) -> Date? {
            unixMs == 0 ? nil : Date(timeIntervalSince1970: TimeInterval(unixMs) / 1000)
        }
        self.init(
            hits: results.hits.map { hit in
                let kind: SearchHit.Kind =
                    switch hit.kind {
                    case .reply: .reply
                    case .toolCall: .toolCall
                    case .toolOutput: .toolOutput
                    case .comment: .comment
                    case .spec: .spec
                    case .message, .unspecified, .UNRECOGNIZED: .message
                    }
                return SearchHit(
                    kind: kind, sessionID: hit.sessionID.isEmpty ? nil : hit.sessionID,
                    sessionName: hit.sessionName, eventID: hit.eventID.isEmpty ? nil : hit.eventID,
                    turn: Int(hit.turn), time: time(hit.timeUnixMs) ?? Date(timeIntervalSince1970: 0),
                    projectID: hit.projectID.isEmpty ? nil : hit.projectID,
                    itemID: hit.itemID.isEmpty ? nil : hit.itemID,
                    threadID: hit.threadID == 0 ? nil : hit.threadID,
                    snippet: SearchHit.runs(hit.snippet), related: hit.related)
            },
            words: results.words, since: time(results.sinceUnixMs), until: time(results.untilUnixMs),
            catchingUp: results.catchingUp)
    }
}

extension PullRequest {
    init(_ pull: Humata_V1_PullRequest) {
        let state: State =
            switch pull.state {
            case .merged: .merged
            case .closed: .closed
            default: .open
            }
        let review: Review =
            switch pull.review {
            case .changesRequested: .changesRequested
            case .commented: .commented
            case .approved: .approved
            default: .none
            }
        let checks: Checks =
            switch pull.checks {
            case .pending: .pending
            case .passing: .passing
            case .failing: .failing
            default: .none
            }
        self.init(
            sessionID: pull.sessionID, itemID: pull.itemID.isEmpty ? nil : pull.itemID, repository: pull.repository,
            githubRepository: pull.githubRepository, number: pull.number, url: pull.url, title: pull.title,
            branch: pull.branch, base: pull.base, state: state, review: review, checks: checks,
            read: date(pull.readUnixMs), problem: pull.problem.isEmpty ? nil : pull.problem)
    }
}

extension AccessGroup {
    /// Nil for a group this app doesn't know, whose actions it leaves out.
    init?(_ group: Humata_V1_AccessGroup) {
        switch group {
        case .read: self = .read
        case .change: self = .change
        case .destroy: self = .destroy
        case .unspecified, .UNRECOGNIZED: return nil
        }
    }
}

extension ClientAccess {
    init(_ access: Humata_V1_ClientAccess) {
        self.init(
            enabled: access.enabled, read: access.read, change: access.change, destroy: access.destroy,
            actions: access.actions.compactMap { action in
                AccessGroup(action.group).map { group in
                    ActionAccess(id: action.id, title: action.title, group: group, enabled: action.enabled)
                }
            })
    }

    /// The switches to keep. An action left out, as one of a group this
    /// app doesn't know, keeps its own.
    var proto: Humata_V1_ClientAccess {
        var access = Humata_V1_ClientAccess()
        access.enabled = enabled
        access.read = read
        access.change = change
        access.destroy = destroy
        access.actions = actions.map { action in
            var kept = Humata_V1_ActionAccess()
            kept.id = action.id
            kept.enabled = action.enabled
            return kept
        }
        return access
    }
}

extension AgentAccess {
    init(_ access: Humata_V1_AgentAccess) {
        self.init(agents: ClientAccess(access.agents), terminal: ClientAccess(access.terminal))
    }

    var proto: Humata_V1_AgentAccess {
        var access = Humata_V1_AgentAccess()
        access.agents = agents.proto
        access.terminal = terminal.proto
        return access
    }
}

extension PullRefresh {
    init(_ refresh: Humata_V1_PullRefresh) {
        let source: GitHubTokenSource =
            switch refresh.tokenSource {
            case .noToken: .missing
            case .commandLine: .commandLine
            case .ghToken: .variable("GH_TOKEN")
            case .githubToken: .variable("GITHUB_TOKEN")
            case .unspecified, .UNRECOGNIZED: .unknown
            }
        self.init(
            automatic: !refresh.manual,
            everyMinutes: refresh.everyMinutes == 0 ? PullRefresh.standard.everyMinutes : Int(refresh.everyMinutes),
            tokenSource: source)
    }

    var proto: Humata_V1_PullRefresh {
        var refresh = Humata_V1_PullRefresh()
        refresh.manual = !automatic
        refresh.everyMinutes = UInt32(clamping: everyMinutes)
        return refresh
    }
}

extension Landed {
    init(_ landed: Humata_V1_SessionLanded) {
        self.init(
            repositories: landed.repositories.map {
                LandedRepository(
                    name: $0.name, branch: $0.branch, commit: $0.commit, base: $0.base, originURL: $0.originURL,
                    pushed: $0.pushed)
            },
            pullRequests: landed.pullRequests.map(PullRequest.init))
    }
}

extension BaseUpdate {
    init(_ updated: Humata_V1_BaseUpdated) {
        self.init(
            repositories: updated.repositories.map { repository in
                let outcome: MergedRepository.Outcome =
                    switch repository.outcome {
                    case .merged: .merged
                    case .conflicts: .conflicts
                    default: .upToDate
                    }
                return MergedRepository(
                    name: repository.name, upstream: repository.upstream, outcome: outcome,
                    conflicts: repository.conflicts)
            },
            sentToAgent: updated.sentToAgent)
    }
}

extension FoundSkill {
    init(_ skill: Humata_V1_FoundSkill) {
        let origin: Origin =
            switch skill.origin {
            case .claudeUser: .claudeUser
            case .copilotUser: .copilotUser
            case .claudeRepository: .claudeRepository
            case .githubRepository: .githubRepository
            case .project, .unspecified, .UNRECOGNIZED: .project
            }
        self.init(
            name: skill.name, description: skill.description_p, path: skill.path, origin: origin,
            projectID: skill.projectID, repository: skill.repository, hash: skill.hash, host: skill.host)
    }
}

extension SkillList {
    init(_ list: Humata_V1_SkillList) {
        self.init(
            skills: list.skills.map(FoundSkill.init),
            clashes: list.clashes.map {
                SkillClash(
                    name: $0.name, candidates: $0.candidates.map(FoundSkill.init),
                    winner: $0.winner.isEmpty ? nil : $0.winner)
            },
            unreachable: list.unreachable)
    }
}

extension Plugin.Agent {
    init?(_ agent: Humata_V1_PluginAgent) {
        switch agent {
        case .claude: self = .claude
        case .copilot: self = .copilot
        case .unspecified, .UNRECOGNIZED: return nil
        }
    }

    var proto: Humata_V1_PluginAgent {
        switch self {
        case .claude: .claude
        case .copilot: .copilot
        }
    }
}

extension Plugin {
    init?(_ plugin: Humata_V1_FoundPlugin) {
        guard let agent = Agent(plugin.agent) else { return nil }
        self.init(
            agent: agent, pluginID: plugin.id, name: plugin.name, description: plugin.description_p,
            version: plugin.version, host: plugin.host, enabled: plugin.enabled, skills: plugin.skills,
            commands: plugin.commands, agents: plugin.agents, mcpServers: plugin.mcpServers, hooks: plugin.hooks,
            lspServers: plugin.lspServers)
    }
}

extension PluginChoice {
    init?(_ choice: Humata_V1_PluginChoice) {
        guard let agent = Plugin.Agent(choice.agent) else { return nil }
        self.init(agent: agent, id: choice.id, enabled: choice.enabled)
    }
}

extension SavedRepository {
    init(_ repository: Humata_V1_SavedRepository) {
        self.init(
            id: repository.id, name: repository.name, location: repository.location,
            source: repository.source == .github ? .github : .used, owner: repository.owner,
            added: date(repository.addedUnixMs),
            lastUsed: repository.lastUsedUnixMs == 0 ? nil : date(repository.lastUsedUnixMs))
    }
}

extension RepositoryOwner {
    init(_ owner: Humata_V1_RepositoryOwner) {
        self.init(
            login: owner.login, added: date(owner.addedUnixMs),
            refreshed: owner.refreshedUnixMs == 0 ? nil : date(owner.refreshedUnixMs),
            error: owner.error.isEmpty ? nil : owner.error, repositories: Int(owner.repositories))
    }
}
