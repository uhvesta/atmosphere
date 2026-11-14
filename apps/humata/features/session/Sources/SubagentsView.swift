import ChatKit
import ChatUI
import ComposableArchitecture
import HumataDaemonClient
import InferenceKit
import SwiftUI

/// A subagent where its tool call sits in the chat: what kind it is and
/// what it was asked, whether it still runs, how long it has, what it
/// used, and, while it runs, what it last did. Clicking opens its chat;
/// its menu also shows what it returned to the agent that started it.
struct SubagentCard: View {
    let run: SubagentRun
    /// What its call returned: the raw result the agent that started it
    /// got. Nil while it runs.
    var result: String?
    var showResult: (String) -> Void = { _ in }
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                SubagentStatusIcon(status: run.status)
                VStack(alignment: .leading, spacing: 3) {
                    SubagentTitle(run: run)
                    if run.status == .running, let step = run.latestStep {
                        Text(step)
                            .scaledFont(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                Spacer(minLength: 8)
                SubagentStats(run: run)
                Image(systemName: "chevron.right")
                    .scaledFont(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .padding(10)
        .background(.fill.quaternary.opacity(0.5), in: .rect(cornerRadius: 12))
        .help("Read what this agent did")
        .accessibilityIdentifier("subagent.card")
        .contextMenu {
            Button("Open the Agent’s Chat", action: open)
            Button("Show Result") {
                if let result { showResult(result) }
            }
            .disabled(result?.isEmpty != false)
        }
    }
}

/// A spinner while the subagent runs, then how it ended.
private struct SubagentStatusIcon: View {
    let status: Subagent.Status

    var body: some View {
        switch status {
        case .running:
            ProgressView()
                .controlSize(.mini)
                .accessibilityLabel("Running")
        case .completed:
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(.green)
                .accessibilityLabel("Finished")
        case .failed:
            Image(systemName: "xmark.circle.fill")
                .foregroundStyle(.red)
                .accessibilityLabel("Failed")
        case .stopped:
            Image(systemName: "stop.circle")
                .foregroundStyle(.secondary)
                .accessibilityLabel("Stopped before it finished")
        }
    }
}

/// The subagent's kind, then what it was asked to do.
private struct SubagentTitle: View {
    let run: SubagentRun

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(run.agentType.isEmpty ? "Agent" : run.agentType)
                .scaledFont(.callout.weight(.semibold))
                .lineLimit(1)
                .layoutPriority(1)
            if !run.description.isEmpty {
                Text(run.description)
                    .scaledFont(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }
}

/// How long the subagent has run, counting while it does, and the tokens
/// and tool calls it used, each only once there's some to show.
private struct SubagentStats: View {
    let run: SubagentRun

    var body: some View {
        HStack(spacing: 4) {
            if let duration = run.duration {
                Text(duration.formatted(.units(allowed: [.hours, .minutes, .seconds], width: .narrow)))
            } else {
                // Counts by itself, so no event is needed to move it on.
                Text(run.started, style: .timer)
            }
            if run.tokens > 0 {
                Text(verbatim: "· \(run.tokens.formatted(.number.notation(.compactName))) tokens")
            }
            if run.toolUses > 0 {
                Text(verbatim: run.toolUses == 1 ? "· 1 tool" : "· \(run.toolUses) tools")
            }
            if run.status == .stopped {
                Text("· stopped")
            } else if run.status == .failed {
                Text("· failed")
            }
        }
        .scaledFont(.caption.monospacedDigit())
        .foregroundStyle(.secondary)
        .lineLimit(1)
        .fixedSize()
    }
}

/// In the header once the agent has started a subagent: how many there
/// are, with a spinner while any runs. Opens their list on the one running
/// last, or else the last. Its own view, so a subagent's step draws only
/// this of the header.
struct AgentsButton: View {
    let store: StoreOf<SessionFeature>
    /// Whether it shows its name beside its icon.
    let names: Bool

    var body: some View {
        let subagents = store.subagents
        if let latest = subagents.last(where: { $0.status == .running }) ?? subagents.last {
            let running = subagents.filter { $0.status == .running }.count
            Button {
                store.send(.subagentTapped(callID: latest.id))
            } label: {
                HStack(spacing: 4) {
                    if running > 0 {
                        ProgressView().controlSize(.mini)
                    } else {
                        Image(systemName: "person.2")
                    }
                    if names {
                        Text(
                            verbatim: running > 0
                                ? "Agents (\(running) of \(subagents.count) running)" : "Agents (\(subagents.count))")
                    }
                }
            }
            .fixedSize()
            .help("The subagents this session's agent started: what each did, how long it took and what it used")
            .accessibilityLabel("Agents")
            .accessibilityIdentifier("session.agents")
        }
    }
}

/// The session's agents listed, the main one first, beside the chat of the
/// subagent picked, to read: nothing is sent from here.
struct SubagentsSheet: View {
    let store: StoreOf<SessionFeature>

    var body: some View {
        HStack(spacing: 0) {
            SubagentList(store: store)
                .frame(width: 300)
            Divider()
            VStack(spacing: 0) {
                SubagentHeader(store: store)
                Divider()
                SubagentChat(store: store)
                    // Each subagent's chat opens at its own end.
                    .id(store.shownSubagent)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(minWidth: 900, idealWidth: 1000, minHeight: 560, idealHeight: 680)
    }
}

/// Main, then each subagent in the order it started, with its status,
/// time, tokens and tool calls. Picking Main goes back to the session's
/// own chat.
private struct SubagentList: View {
    let store: StoreOf<SessionFeature>

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 2) {
                Button {
                    store.send(.subagentClosed)
                } label: {
                    MainAgentRow(store: store)
                }
                .buttonStyle(.plain)
                .help("Back to the session's own chat")
                ForEach(store.subagents) { run in
                    Button {
                        store.send(.subagentTapped(callID: run.id))
                    } label: {
                        SubagentListRow(run: run, picked: run.id == store.shownSubagent)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(8)
        }
        .background(.fill.quinary)
    }
}

/// The session's own agent in the list: whether it's working, and what the
/// session has used.
private struct MainAgentRow: View {
    let store: StoreOf<SessionFeature>

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            if store.isWaiting {
                // It stands still until the person answers, under this sheet.
                Image(systemName: "hand.raised.fill")
                    .foregroundStyle(.orange)
            } else if store.isWorking {
                ProgressView().controlSize(.mini)
            } else {
                Image(systemName: "bubble.left.and.text.bubble.right")
                    .foregroundStyle(.secondary)
            }
            VStack(alignment: .leading, spacing: 3) {
                Text("Main")
                    .scaledFont(.callout.weight(.semibold))
                if store.isWaiting {
                    Text("Waiting for you: answer in the chat")
                        .scaledFont(.caption)
                        .foregroundStyle(.secondary)
                }
                if let usage = store.usage {
                    Text(SessionView.describe(usage))
                        .scaledFont(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(8)
        .contentShape(.rect)
    }
}

private struct SubagentListRow: View {
    let run: SubagentRun
    let picked: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            SubagentStatusIcon(status: run.status)
            VStack(alignment: .leading, spacing: 3) {
                Text(run.agentType.isEmpty ? "Agent" : run.agentType)
                    .scaledFont(.callout.weight(.semibold))
                    .lineLimit(1)
                if !run.description.isEmpty {
                    Text(run.description)
                        .scaledFont(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                SubagentStats(run: run)
            }
            Spacer(minLength: 0)
        }
        .padding(8)
        .background(picked ? Color.accentColor.opacity(0.18) : Color.clear, in: .rect(cornerRadius: 8))
        .contentShape(.rect)
        .accessibilityAddTraits(picked ? .isSelected : [])
    }
}

/// Over the open subagent's chat: what it is, how it's going, and Done.
private struct SubagentHeader: View {
    let store: StoreOf<SessionFeature>

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            if let run = store.shownSubagent.flatMap({ store.subagents[id: $0] }) {
                SubagentStatusIcon(status: run.status)
                SubagentTitle(run: run)
                Spacer(minLength: 8)
                SubagentStats(run: run)
            } else {
                Spacer()
            }
            Button("Done") { store.send(.subagentClosed) }
                .keyboardShortcut(.cancelAction)
        }
        .padding(.horizontal)
        .padding(.vertical, 10)
    }
}

/// The open subagent's chat, read only: its messages, thoughts and tool
/// calls as the session's chat shows its own, kept at its end as it grows.
private struct SubagentChat: View {
    let store: StoreOf<SessionFeature>
    @State private var openThinking: Set<String> = []
    @State private var openTools: Set<String> = []
    @State private var selecting: SelectedText?

    var body: some View {
        let items = store.subagentItems
        let select: (String) -> Void = { selecting = SelectedText(text: $0) }
        if items.isEmpty {
            ContentUnavailableView(
                "No Steps Yet", systemImage: "person.2",
                description: Text(
                    "This agent's own steps show here as it takes them. What it returns goes to the main agent, "
                        + "under its call in the chat.")
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    ForEach(items) { item in
                        SubagentChatRow(
                            item: item, thinkingOpen: openThinking.contains(item.id),
                            toolOpen: openTools.contains(item.id), nested: nested(item),
                            toggleThinking: { openThinking.formSymmetricDifference([item.id]) },
                            toggleTool: { openTools.formSymmetricDifference([item.id]) },
                            open: { store.send(.subagentTapped(callID: $0)) }, select: select
                        )
                        .equatable()
                    }
                }
                .padding()
            }
            .defaultScrollAnchor(.bottom)
            .sheet(item: $selecting) { SelectTextSheet(text: $0.text) }
        }
    }

    /// The subagent `item`'s tool call started itself, if it did.
    private func nested(_ item: ChatItem) -> SubagentRun? {
        guard case .tool(let call) = item else { return nil }
        return store.subagents[id: call.callID]
    }
}

private struct SelectedText: Identifiable {
    let id = UUID()
    let text: String
}

/// One row of a subagent's chat. Equal when what it shows is, so a new
/// step draws only its own row.
private struct SubagentChatRow: View, Equatable {
    let item: ChatItem
    let thinkingOpen: Bool
    let toolOpen: Bool
    /// The subagent the row's tool call started.
    let nested: SubagentRun?
    let toggleThinking: () -> Void
    let toggleTool: () -> Void
    let open: (String) -> Void
    let select: (String) -> Void

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.item == rhs.item && lhs.thinkingOpen == rhs.thinkingOpen && lhs.toolOpen == rhs.toolOpen
            && lhs.nested == rhs.nested
    }

    var body: some View {
        switch item {
        case .person(_, let text, _):
            MessageView(role: .person, text: text, select: select)
        case .fromSession(_, let sender):
            SentBySession(sender: sender, select: select)
        case .reply(_, let text):
            MessageView(role: .assistant, text: text, select: select)
        case .thinking(_, let text):
            ThinkingDisclosure(
                thinking: ModelThinking(text: text), isExpanded: thinkingOpen, toggle: toggleThinking,
                select: select, answering: true)
        case .tool(let call):
            if let nested {
                SubagentCard(run: nested, result: call.output, showResult: select) { open(call.callID) }
            } else {
                ToolCallView(call: call, expanded: toolOpen, toggle: toggleTool, select: select)
            }
        case .permission(_, let permission):
            PermissionCard(permission: permission, select: select)
        case .compacted:
            Label(
                "Context compacted: the agent summed up what came before to make room",
                systemImage: "rectangle.compress.vertical"
            )
            .scaledFont(.footnote)
            .foregroundStyle(.secondary)
        case .notice(_, let text):
            Label(text, systemImage: "info.circle")
                .scaledFont(.footnote)
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
        case .failure(_, let message):
            Label(message, systemImage: "exclamationmark.triangle")
                .scaledFont(.footnote)
                .foregroundStyle(.red)
                .textSelection(.enabled)
        // Turns end, stop and are rewound for the session, not a subagent.
        case .stopped, .cost, .rewound:
            EmptyView()
        }
    }
}
