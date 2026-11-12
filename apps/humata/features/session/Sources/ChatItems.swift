import Foundation
import HumataDaemonClient

/// One thing the chat shows, worked out from a session's events: a message,
/// what the agent thought, a tool it ran with the permission it asked for,
/// or how a turn ended badly.
package enum ChatItem: Identifiable, Hashable, Sendable {
    case person(id: String, text: String)
    case reply(id: String, text: String)
    case thinking(id: String, text: String)
    case tool(ToolCall)
    /// A permission asked for outside any tool call.
    case permission(id: String, Permission)
    /// The person stopped the turn.
    case stopped(id: String)
    case failure(id: String, message: String)

    package var id: String {
        switch self {
        case .person(let id, _), .reply(let id, _), .thinking(let id, _), .permission(let id, _), .stopped(let id),
            .failure(let id, _):
            id
        case .tool(let call): call.id
        }
    }
}

package struct ToolCall: Hashable, Sendable {
    package var id: String
    package var tool: String
    /// The command it runs or the file it writes, when its arguments say;
    /// otherwise nil.
    package var subject: String?
    /// Nil while it runs.
    package var output: String?
    package var succeeded: Bool?
    package var permission: Permission?

    /// Reads the command or path from a call's arguments.
    static func subject(of argumentsJSON: String) -> String? {
        guard let data = argumentsJSON.data(using: .utf8),
            let arguments = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        let keys = ["command", "file_path", "path", "notebook_path", "pattern", "url", "query", "description"]
        return keys.lazy.compactMap { arguments[$0] as? String }.first { !$0.isEmpty }
    }
}

package struct Permission: Hashable, Sendable {
    package enum Answer: Hashable, Sendable {
        case approved
        case denied
        /// The session's approval mode allowed it.
        case allowedByMode
    }

    package var requestID: String
    package var kind: PermissionKind
    package var summary: String
    /// Nil while the agent waits for it.
    package var answer: Answer?
}

/// What a session has used so far.
package struct Usage: Hashable, Sendable {
    /// The model of the latest turn.
    package var model: String
    package var inputTokens: UInt64 = 0
    package var outputTokens: UInt64 = 0
    package var premiumRequests: Double = 0

    package init(model: String, inputTokens: UInt64 = 0, outputTokens: UInt64 = 0, premiumRequests: Double = 0) {
        self.model = model
        self.inputTokens = inputTokens
        self.outputTokens = outputTokens
        self.premiumRequests = premiumRequests
    }
}

extension ChatItem {
    /// The chat for `events`, oldest first. A permission request belongs to
    /// the tool call running when it was made.
    package static func items(_ events: some Sequence<SessionEvent>) -> [ChatItem] {
        var items: [ChatItem] = []
        var calls: [String: Int] = [:]
        var running: String?
        var requests: [String: Int] = [:]

        func answer(_ request: String, _ answer: Permission.Answer) {
            guard let index = requests[request] else { return }
            switch items[index] {
            case .tool(var call):
                call.permission?.answer = answer
                items[index] = .tool(call)
            case .permission(let id, var permission):
                permission.answer = answer
                items[index] = .permission(id: id, permission)
            default: break
            }
        }

        for event in events {
            switch event.kind {
            case .userMessage(let text):
                items.append(.person(id: event.id, text: text))
            case .assistantMessage(let text):
                items.append(.reply(id: event.id, text: text))
            case .reasoning(let text):
                if case .thinking(let id, let before)? = items.last {
                    items[items.count - 1] = .thinking(id: id, text: before + text)
                } else {
                    items.append(.thinking(id: event.id, text: text))
                }
            case .toolStarted(let callID, let tool, let arguments):
                calls[callID] = items.count
                running = callID
                items.append(
                    .tool(ToolCall(id: callID, tool: tool, subject: ToolCall.subject(of: arguments))))
            case .toolCompleted(let callID, let success, let output):
                if running == callID { running = nil }
                guard let index = calls[callID], case .tool(var call) = items[index] else { continue }
                call.output = output
                call.succeeded = success
                items[index] = .tool(call)
            case .permissionRequested(let request, let kind, let summary):
                let permission = Permission(requestID: request, kind: kind, summary: summary)
                if let running, let index = calls[running], case .tool(var call) = items[index],
                    call.permission == nil
                {
                    call.permission = permission
                    items[index] = .tool(call)
                    requests[request] = index
                } else {
                    requests[request] = items.count
                    items.append(.permission(id: event.id, permission))
                }
            case .permissionCompleted(let request, let approved, let automatic):
                answer(request, automatic ? .allowedByMode : approved ? .approved : .denied)
            case .turnEnded(aborted: true):
                running = nil
                items.append(.stopped(id: event.id))
            case .failed(let message):
                items.append(.failure(id: event.id, message: message))
            case .turnStarted, .assistantDelta, .turnEnded, .idle, .usage, .checkpoint:
                continue
            }
        }
        return items
    }
}

extension Usage {
    /// Everything `events` report used; nil if they report nothing.
    package static func total(_ events: some Sequence<SessionEvent>) -> Usage? {
        var usage: Usage?
        for event in events {
            guard case .usage(let model, let input, let output, let premium) = event.kind else { continue }
            var total = usage ?? Usage(model: model)
            total.model = model
            total.inputTokens += input
            total.outputTokens += output
            total.premiumRequests += premium
            usage = total
        }
        return usage
    }
}
