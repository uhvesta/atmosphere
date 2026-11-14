import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// A subagent the session's agent started with a tool call: what its
/// backend last reported of it, and what its own steps add up to.
package struct SubagentRun: Identifiable, Hashable, Sendable {
    /// The tool call that started it.
    package var id: String
    /// Such as general-purpose or Explore; empty when the backend doesn't
    /// say.
    package var agentType = ""
    /// What it was asked to do, in a few words; empty when not said.
    package var description = ""
    /// One still running when its turn ended reads as stopped.
    package var status = Subagent.Status.running
    /// When its first event happened.
    package var started: Date
    /// When its last event happened, once it stopped; nil while it runs,
    /// so a step that shows nothing new leaves the run as it was.
    package var ended: Date?
    /// What its backend reports it used; 0 when it hasn't said.
    var reportedTokens: UInt64 = 0
    var reportedToolUses: UInt64 = 0
    var reportedDuration = Duration.zero
    /// The tool calls among its steps.
    var calls: UInt64 = 0
    /// What its own steps report used; nil if they report nothing.
    package var usage: Usage?
    /// What it last did: the command or file of its last tool call, or
    /// the start of what it last said.
    package var latestStep: String?

    /// Tokens in and out so far: what its backend reports, or else what
    /// its steps add up to.
    package var tokens: UInt64 {
        reportedTokens > 0 ? reportedTokens : (usage?.inputTokens ?? 0) + (usage?.outputTokens ?? 0)
    }

    /// How many tools it called: what its backend reports, or else the
    /// calls among its steps.
    package var toolUses: UInt64 { max(reportedToolUses, calls) }

    /// How long it ran, once it stopped: what its backend reports, or else
    /// from its first event to its last. Nil while it runs, when it's
    /// counted from `started`.
    package var duration: Duration? {
        guard status != .running else { return nil }
        if reportedDuration > .zero { return reportedDuration }
        return .milliseconds(Int64(max(0, (ended ?? started).timeIntervalSince(started)) * 1000))
    }
}

/// The session's subagents, worked out an event at a time as the chat is.
/// Each has a chat of its own, made of the events that name its call.
struct SubagentLog: Equatable, Sendable {
    /// In the order they started.
    private(set) var runs: IdentifiedArrayOf<SubagentRun> = []
    private var chats: [String: Chat] = [:]
    /// Everything the subagents' own steps report used; nil if they report
    /// nothing.
    private(set) var usage: Usage?
    /// When each subagent's latest event happened, by its call. Kept here
    /// and not in its run, which what draws it compares whole.
    private var latest: [String: Date] = [:]

    /// What `call`'s subagent did, oldest first.
    func items(of call: String) -> [ChatItem] { chats[call]?.items ?? [] }

    /// Takes in an event of the subagent `call` started.
    mutating func add(_ event: SessionEvent, under call: String) {
        var run = runs[id: call] ?? SubagentRun(id: call, started: event.time)
        let latest = self.latest[call].map { max($0, event.time) } ?? event.time
        if self.latest[call] != latest { self.latest[call] = latest }
        switch event.kind {
        case .subagent(let reported):
            if !reported.agentType.isEmpty { run.agentType = reported.agentType }
            if !reported.description.isEmpty { run.description = reported.description }
            run.status = reported.status
            run.reportedTokens = reported.totalTokens
            run.reportedToolUses = reported.toolUses
            run.reportedDuration = reported.duration
        case .assistantDelta:
            // The whole message comes after its parts, and a subagent's
            // chat shows it then: nothing is worked out for each word of
            // a subagent nobody may be reading.
            return
        default:
            // Taken out to change, so it isn't copied whole for an event.
            var chat = chats.removeValue(forKey: call) ?? Chat()
            chat.add(event)
            run.usage = chat.usage
            if case .usage(let model, let input, let output, let premium) = event.kind {
                var total = usage ?? Usage(model: model)
                total.model = model
                total.inputTokens += input
                total.outputTokens += output
                total.premiumRequests += premium
                usage = total
            }
            if case .toolStarted = event.kind { run.calls += 1 }
            if let step = Self.step(event, in: chat) { run.latestStep = step }
            chats[call] = chat
        }
        // Its run ends at its last event only once it's no longer running.
        let ended = run.status == .running ? nil : latest
        if run.ended != ended { run.ended = ended }
        runs[id: call] = run
    }

    /// The session's turn ended, and its subagents with it: one whose
    /// backend never said it finished didn't.
    mutating func turnEnded() {
        for id in runs.ids where runs[id: id]?.status == .running {
            runs[id: id]?.status = .stopped
            runs[id: id]?.ended = latest[id]
        }
    }

    /// What `event` says the subagent is doing, as its row shows in a line.
    private static func step(_ event: SessionEvent, in chat: Chat) -> String? {
        switch event.kind {
        case .toolStarted(_, let tool, _):
            guard case .tool(let call)? = chat.items.last else { return tool }
            return call.subject ?? tool
        case .assistantMessage(let text):
            let line = text.split(whereSeparator: \.isNewline).first.map(String.init) ?? ""
            return line.isEmpty ? nil : String(line.prefix(200))
        default:
            return nil
        }
    }
}

extension Usage {
    /// This and `other` together, under this one's model.
    func adding(_ other: Usage) -> Usage {
        var total = self
        total.inputTokens += other.inputTokens
        total.outputTokens += other.outputTokens
        total.premiumRequests += other.premiumRequests
        return total
    }
}
