import Foundation
import KhodrDaemonClient
import KhodrProto

extension ErrorCode {
    init(_ code: Khodr_V1_ErrorCode) {
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
            case .unspecified, .UNRECOGNIZED: .unknown
            }
    }
}

extension Backend {
    init?(_ backend: Khodr_V1_Backend) {
        switch backend {
        case .scripted: self = .scripted
        case .local: self = .local
        case .copilot: self = .copilot
        case .unspecified, .UNRECOGNIZED: return nil
        }
    }

    var proto: Khodr_V1_Backend {
        switch self {
        case .scripted: .scripted
        case .local: .local
        case .copilot: .copilot
        }
    }
}

extension ApprovalMode {
    init(_ mode: Khodr_V1_ApprovalMode) {
        self =
            switch mode {
            case .allowEdits: .allowEdits
            case .allowEverything: .allowEverything
            case .askEveryTime, .unspecified, .UNRECOGNIZED: .askEveryTime
            }
    }

    var proto: Khodr_V1_ApprovalMode {
        switch self {
        case .askEveryTime: .askEveryTime
        case .allowEdits: .allowEdits
        case .allowEverything: .allowEverything
        }
    }
}

extension Repository {
    init(_ repository: Khodr_V1_Repository) {
        self.init(name: repository.name, path: repository.path)
    }

    var proto: Khodr_V1_Repository {
        var repository = Khodr_V1_Repository()
        repository.name = name
        repository.path = path
        return repository
    }
}

extension Project {
    init(_ project: Khodr_V1_Project) {
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
    init(_ info: Khodr_V1_SessionInfo) throws {
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
            archived: info.archived, keeper: info.keeper)
    }
}

extension SessionEvent {
    /// Nil for a kind of event this app doesn't know, from a newer daemon.
    init?(_ event: Khodr_V1_SessionEvent) {
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
            let permission: PermissionKind =
                switch request.kind {
                case .shell: .shell
                case .write: .write
                case .read: .read
                case .other, .unspecified, .UNRECOGNIZED: .other
                }
            kind = .permissionRequested(requestID: request.requestID, kind: permission, summary: request.summary)
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
        case nil: return nil
        }
        self.init(id: event.id, time: Date(timeIntervalSince1970: Double(event.timeUnixMs) / 1000), kind: kind)
    }
}
