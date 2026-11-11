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
            forkedBeforeEventID: optional(info.forkedBeforeEventID), host: optional(info.host))
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
            kind = .permissionCompleted(requestID: completed.requestID, approved: completed.approved)
        case .turnEnded(let ended): kind = .turnEnded(aborted: ended.aborted)
        case .idle: kind = .idle
        case .failed(let failed): kind = .failed(failed.message)
        case .usage(let usage):
            kind = .usage(
                model: usage.model, inputTokens: usage.inputTokens, outputTokens: usage.outputTokens,
                premiumRequests: usage.premiumRequests)
        case nil: return nil
        }
        self.init(id: event.id, time: Date(timeIntervalSince1970: Double(event.timeUnixMs) / 1000), kind: kind)
    }
}
