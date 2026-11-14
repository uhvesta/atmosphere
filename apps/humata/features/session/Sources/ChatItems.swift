import ChatUI
import Foundation
import HumataDaemonClient

/// One thing the chat shows, worked out from a session's events: a message,
/// what the agent thought, a tool it ran with the permission it asked for,
/// or how a turn ended badly.
package enum ChatItem: Identifiable, Hashable, Sendable {
    /// What the person sent, with the names of any images sent with it.
    case person(id: String, text: String, images: [String] = [])
    /// What another session's agent sent, with its send_message tool or as
    /// the first message of a session it started: not the person's words.
    case fromSession(id: String, AgentSender)
    case reply(id: String, text: String)
    case thinking(id: String, text: String)
    case tool(ToolCall)
    /// A permission asked for outside any tool call.
    case permission(id: String, Permission)
    /// The person stopped the turn.
    case stopped(id: String)
    case failure(id: String, message: String)
    /// Said by Humata, not the agent.
    case notice(id: String, text: String)
    /// What a turn used, where it ended; only for turns that report it.
    case cost(id: String, Usage)
    /// The chat was cut back here, and with `files` its code too;
    /// `undoable` until the next message.
    case rewound(id: String, files: Bool, undoable: Bool)
    /// The agent summed up what came before to make room.
    case compacted(id: String)

    package var id: String {
        switch self {
        case .person(let id, _, _), .fromSession(let id, _), .reply(let id, _), .thinking(let id, _),
            .permission(let id, _), .stopped(let id),
            .failure(let id, _), .notice(let id, _), .cost(let id, _), .rewound(let id, _, _), .compacted(let id):
            id
        case .tool(let call): call.id
        }
    }
}

package struct ToolCall: Hashable, Sendable {
    /// The row's own id: the event that started the call. Not the agent's
    /// id for it, which can come again in a later turn: two rows with one
    /// id in the chat's lazy list leave one of them blank.
    package var id: String
    /// The agent's id for the call, that its output and proposals name.
    package var callID: String
    package var tool: String
    /// The command it runs or the file it writes, when its arguments say;
    /// otherwise nil.
    package var subject: String?
    /// Nil while it runs.
    package var output: String? {
        didSet { preview = output.map { TextPreview($0) } }
    }
    /// What the chat shows of the output, worked out once when it comes,
    /// not each time the row is drawn.
    package private(set) var preview: TextPreview?
    package var succeeded: Bool?
    package var permission: Permission?
    /// What it writes to its file, when its arguments say; nil for calls
    /// that write nothing.
    package var change: Change?

    package init(
        id: String, callID: String? = nil, tool: String, subject: String? = nil, output: String? = nil,
        succeeded: Bool? = nil,
        permission: Permission? = nil, change: Change? = nil
    ) {
        self.id = id
        self.callID = callID ?? id
        self.tool = tool
        self.subject = subject
        self.output = output
        preview = output.map { TextPreview($0) }
        self.succeeded = succeeded
        self.permission = permission
        self.change = change
    }

    /// The lines a write takes out and puts in, so it can be judged before
    /// it is allowed. A whole-file write puts every line in. Only the first
    /// lines are kept, each cut short, with how many there were: a written
    /// file can be megabytes, and the chat shows no more of it.
    package struct Change: Hashable, Sendable {
        /// Lines kept across removed and added, the removed ones first.
        static let lines = 300
        /// Characters of a line kept: a minified file's one line can be
        /// megabytes.
        static let lineLength = 500

        /// The lines kept.
        package private(set) var removed: [String]
        package private(set) var added: [String]
        /// How many lines it takes out and puts in, kept or not.
        package private(set) var removedCount: Int
        package private(set) var addedCount: Int

        package init(removed: [String], added: [String]) {
            self.init(cutting: removed, added)
        }

        /// Keeps the first lines of `removed`, then of `added`, copying
        /// only those out of the text they are part of.
        fileprivate init<Removed: Collection, Added: Collection>(cutting removed: Removed, _ added: Added)
        where Removed.Element: StringProtocol, Added.Element: StringProtocol {
            removedCount = removed.count
            addedCount = added.count
            let keptRemoved = removed.prefix(Self.lines).map { Self.shortened($0) }
            self.removed = keptRemoved
            self.added = added.prefix(Self.lines - keptRemoved.count).map { Self.shortened($0) }
        }

        private static func shortened<Line: StringProtocol>(_ line: Line) -> String {
            guard line.utf8.count > lineLength else { return String(line) }
            let start = line.prefix(lineLength)
            return start.endIndex < line.endIndex ? String(start) + "…" : String(line)
        }

        /// The lines going from `old` to `new` takes out and puts in.
        package static func between(_ old: String, _ new: String) -> Change {
            let old = old.isEmpty ? [] : old.components(separatedBy: "\n")
            let new = new.isEmpty ? [] : new.components(separatedBy: "\n")
            var removed: [String] = []
            var added: [String] = []
            for step in new.difference(from: old) {
                switch step {
                case .remove(_, let line, _): removed.append(line)
                case .insert(_, let line, _): added.append(line)
                }
            }
            return Change(removed: removed, added: added)
        }
    }

    /// Reads from a call's arguments, parsed once, the command or path it
    /// names and what it writes: a replacement (Claude's Edit and
    /// MultiEdit, Copilot's edit) or a whole file (Write, create,
    /// write_file).
    static func parts(of argumentsJSON: String) -> (subject: String?, change: Change?) {
        guard let data = argumentsJSON.data(using: .utf8),
            let arguments = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return (nil, nil) }
        let keys = ["command", "file_path", "path", "notebook_path", "pattern", "url", "query", "description"]
        let subject = keys.lazy.compactMap { arguments[$0] as? String }.first { !$0.isEmpty }
        return (subject, change(in: arguments))
    }

    private static func change(in arguments: [String: Any]) -> Change? {
        func lines(_ text: String?) -> [Substring] {
            guard let text, !text.isEmpty else { return [] }
            return text.split(separator: "\n", omittingEmptySubsequences: false)
        }
        func replacement(_ edit: [String: Any]) -> (removed: [Substring], added: [Substring])? {
            let old = (edit["old_string"] ?? edit["old_str"]) as? String
            let new = (edit["new_string"] ?? edit["new_str"]) as? String
            guard old != nil || new != nil else { return nil }
            return (lines(old), lines(new))
        }
        if let edits = arguments["edits"] as? [[String: Any]] {
            let changes = edits.compactMap(replacement)
            guard !changes.isEmpty else { return nil }
            return Change(cutting: changes.flatMap { $0.removed }, changes.flatMap { $0.added })
        }
        if let change = replacement(arguments) { return Change(cutting: change.removed, change.added) }
        let whole = ["content", "contents", "file_text"].lazy.compactMap { arguments[$0] as? String }.first
        return whole.map { Change(cutting: [Substring](), lines($0)) }
    }
}

package struct Permission: Hashable, Sendable {
    package enum Answer: Hashable, Sendable {
        case approved
        case denied
        /// The session's approval mode allowed it.
        case allowedByMode
        /// The agent's own judge let it through, in Auto.
        case allowedByJudge
        /// Its turn ended, stopped or cut short, before it was answered.
        case notAnswered
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
    /// the tool call running when it was made. A subagent's steps aren't
    /// the chat's.
    package static func items(_ events: some Sequence<SessionEvent>) -> [ChatItem] {
        var chat = Chat()
        for event in events where event.parentCallID == nil { chat.add(event) }
        return chat.items
    }
}

/// The chat worked out an event at a time, so a live event is added to it
/// instead of the chat being worked out again from the start. A value of
/// values, so a history's chat can be worked out off the main thread and
/// handed to it.
struct Chat: Equatable, Sendable {
    private(set) var items: [ChatItem] = []
    private var calls: [String: Int] = [:]
    private var running: String?
    private var requests: [String: Int] = [:]
    /// The usage reported in the turn so far.
    private var turn: [SessionEvent] = []
    private var rewound: Int?

    /// Whether a turn has started and not yet ended.
    private(set) var working = false
    /// The person's last message.
    private(set) var lastMessage: String?
    /// The names of the images sent with it.
    private(set) var lastImages: [String] = []
    /// Whether the last turn failed and nothing has started since.
    private(set) var lastTurnFailed = false
    /// Whether the last turn asked for was to sum up the conversation, not
    /// to answer a message: it holds no message to send again or edit.
    private(set) var compacting = false
    /// The newest failure's row.
    private(set) var lastFailure: String?
    /// Everything the session's turns report used; nil if they report nothing.
    private(set) var usage: Usage?
    /// Requests for permission not answered yet, whose turn hasn't ended.
    private(set) var pendingPermissions: [SessionEvent] = []
    /// Messages waiting for their own turn, first to run first.
    private(set) var queued: [QueuedMessage] = []
    /// Queued messages that never ran, since the person last sent one.
    private(set) var dropped: [QueuedMessage] = []
    private(set) var turnsStarted = 0
    private(set) var turnsEnded = 0
    /// The tool calls proposing a task or a spec edit.
    private(set) var proposals: Set<String> = []

    /// Keeps what the events add up to, as each comes, so nothing reads
    /// the whole history again for a new event.
    private mutating func tally(_ event: SessionEvent) {
        switch event.kind {
        case .turnStarted:
            working = true
            lastTurnFailed = false
            turnsStarted += 1
        case .turnEnded:
            working = false
            turnsEnded += 1
            pendingPermissions = []
        case .idle:
            working = false
        case .userMessage(let text, let images):
            lastMessage = text
            lastImages = images
            lastTurnFailed = false
            compacting = false
            dropped = []
        case .compacting:
            compacting = true
            lastTurnFailed = false
        case .failed:
            lastTurnFailed = true
            lastFailure = event.id
        case .usage(let model, let input, let output, let premium):
            var total = usage ?? Usage(model: model)
            total.model = model
            total.inputTokens += input
            total.outputTokens += output
            total.premiumRequests += premium
            usage = total
        case .permissionRequested:
            pendingPermissions.append(event)
        case .permissionCompleted(let request, _, _, _):
            pendingPermissions.removeAll {
                if case .permissionRequested(request, _, _) = $0.kind { true } else { false }
            }
        case .messageQueued(let id, let text, let images):
            // Again for one still queued: it was edited, or joined.
            if let index = queued.firstIndex(where: { $0.id == id }) {
                queued[index].text = text
                queued[index].images = images
            } else {
                queued.append(QueuedMessage(id: id, text: text, images: images))
            }
        case .dequeued(let id, let wasDropped):
            if wasDropped, let message = queued.first(where: { $0.id == id }) { dropped.append(message) }
            queued.removeAll { $0.id == id }
        case .toolStarted(let callID, let tool, _):
            if tool.hasSuffix("propose_task") || tool.hasSuffix("propose_spec_edit") { proposals.insert(callID) }
        default:
            break
        }
    }

    private mutating func answer(_ request: String, _ answer: Permission.Answer) {
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

    /// The last rewind can't be undone once a message is sent or it was.
    private mutating func settleRewind() {
        guard let index = rewound, case .rewound(let id, let files, _) = items[index] else { return }
        items[index] = .rewound(id: id, files: files, undoable: false)
        rewound = nil
    }

    mutating func add(_ event: SessionEvent) {
        tally(event)
        if case .usage = event.kind { turn.append(event) }
        if case .turnEnded = event.kind {
            defer { turn = [] }
            // Nothing waits on a request once its turn is over.
            for request in requests.keys { answer(request, .notAnswered) }
            requests = [:]
            if case .turnEnded(aborted: true) = event.kind {
                running = nil
                items.append(.stopped(id: event.id))
            }
            if let usage = Usage.total(turn) { items.append(.cost(id: "\(event.id)-cost", usage)) }
            return
        }
        switch event.kind {
        case .userMessage(let text, let images):
            settleRewind()
            if let sender = AgentSender(text) {
                items.append(.fromSession(id: event.id, sender))
            } else {
                items.append(.person(id: event.id, text: text, images: images))
            }
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
            let parts = ToolCall.parts(of: arguments)
            items.append(
                .tool(ToolCall(id: event.id, callID: callID, tool: tool, subject: parts.subject, change: parts.change)))
        case .toolCompleted(let callID, let success, let output):
            if running == callID { running = nil }
            guard let index = calls[callID], case .tool(var call) = items[index] else { return }
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
        case .permissionCompleted(let request, let approved, let automatic, let judged):
            answer(
                request,
                judged ? .allowedByJudge : automatic ? .allowedByMode : approved ? .approved : .denied)
            requests[request] = nil
        case .failed(let message):
            items.append(.failure(id: event.id, message: message))
        case .notice(let text):
            items.append(.notice(id: event.id, text: text))
        case .rewound(_, _, let files):
            settleRewind()
            rewound = items.count
            items.append(.rewound(id: event.id, files: files, undoable: true))
        case .rewindUndone:
            settleRewind()
        case .compacted:
            items.append(.compacted(id: event.id))
        case .compacting:
            // The turn it starts shows it; like a message, it ends the
            // chance to undo a rewind.
            settleRewind()
        case .turnStarted, .assistantDelta, .turnEnded, .idle, .usage, .checkpoint, .messageQueued, .dequeued,
            .waiting, .subagent:
            return
        }
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

/// A message waiting for its own turn.
package struct QueuedMessage: Identifiable, Hashable, Sendable {
    package var id: String
    package var text: String
    /// Names of the images sent with it.
    package var images: [String]

    package init(id: String, text: String, images: [String] = []) {
        self.id = id
        self.text = text
        self.images = images
    }
}
