import AppKit
import ChatKit
import ChatUI
import ComposableArchitecture
import HumataDaemonClient
import HumataDocs
import HumataReview
import HumataTerminal
import InferenceKit
import SwiftUI

/// A session's chat: the person's messages, the agent's thinking, replies,
/// the commands it runs with their output and the permissions it asks for,
/// with the field to send, steer or queue.
package struct SessionView: View {
    @Bindable var store: StoreOf<SessionFeature>
    @State private var selecting: TextToSelect?

    package init(store: StoreOf<SessionFeature>) {
        self.store = store
    }

    package var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            if store.tab == .review {
                ReviewView(store: store.scope(state: \.review, action: \.review))
            } else if store.tab == .files {
                DocsView(store: store.scope(state: \.docs, action: \.docs))
            } else if store.tab == .terminal {
                TerminalsView(store: store.scope(state: \.terminals, action: \.terminals))
            } else {
                chat
                    .alert($store.scope(state: \.proposals.alert, action: \.proposals.alert))
            }
            if store.disconnected, !store.session.hostOffline {
                ChatNotice(
                    "Lost the connection to Humata's daemon. Reconnecting…", systemImage: "bolt.horizontal.circle",
                    action: "Reconnect Now"
                ) { store.send(.task) }
            }
            if store.session.hostOffline, let host = store.session.host {
                ChatNotice(
                    LocalizedStringKey(
                        "Can't reach \(host), so this is the session as it last was. Humata reconnects when you "
                            + "send a message; check \(host) in Hosts if it doesn't."),
                    systemImage: "wifi.slash")
            }
            if store.session.archived {
                Label(
                    "This session is archived. Unarchive it from the sidebar to work in it again; its code comes "
                        + "back as it was.", systemImage: "archivebox"
                )
                .font(.callout)
                .foregroundStyle(.secondary)
                .padding()
            } else if store.tab == .chat {
                composer
                    .padding([.horizontal, .bottom])
            }
        }
        .navigationTitle(store.session.name.isEmpty ? "Session" : store.session.name)
        .task { await store.send(.task).finish() }
        .alert($store.scope(state: \.alert, action: \.alert))
        .sheet(item: $store.scope(state: \.land, action: \.land)) { LandView(store: $0) }
        .sheet(item: $selecting) { SelectTextSheet(text: $0.text) }
        .sheet(isPresented: editingProposal) {
            ProposalEditor(store: store.scope(state: \.proposals, action: \.proposals))
        }
    }

    private var editingProposal: Binding<Bool> {
        Binding(
            get: { store.proposals.editing != nil },
            set: { if !$0 { store.send(.proposals(.editCancelled)) } })
    }

    private func title(_ tab: SessionFeature.Tab) -> String {
        switch tab {
        case .chat: "Chat"
        case .review: store.review.pending > 0 ? "Review (\(store.review.pending))" : "Review"
        case .files: store.docs.drafts.isEmpty ? "Files" : "Files (\(store.docs.drafts.count))"
        case .terminal: "Terminal"
        }
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(store.session.name.isEmpty ? "Session" : store.session.name)
                    .font(.headline)
                    .lineLimit(1)
                Text(engine)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            .frame(minWidth: 90, alignment: .leading)
            Spacer()
            Picker("View", selection: Binding(get: { store.tab }, set: { store.send(.tabPicked($0)) })) {
                ForEach(store.tabs, id: \.self) { tab in
                    Text(title(tab)).tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .fixedSize()
            .help("The chat, the session's changes to review, the files in its folder, or a terminal there (⌘1 to ⌘4)")
            .background { tabShortcuts }
            Spacer()
            if let usage = store.usage {
                Text(Self.describe(usage))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .help("What this session has used")
            }
            ViewThatFits(in: .horizontal) {
                actions.labelStyle(.titleAndIcon)
                actions.labelStyle(.iconOnly)
            }
        }
        .padding(.horizontal)
        .padding(.vertical, 10)
    }

    /// Fork, Ship, Folder and the approval mode; icons only when the
    /// window is too narrow for their names.
    private var actions: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Menu {
                Button("Fork With the Code") { store.send(.forkTapped(before: nil, chatOnly: false)) }
                Button("Fork the Chat Only") { store.send(.forkTapped(before: nil, chatOnly: true)) }
            } label: {
                Label("Fork", systemImage: "arrow.triangle.branch")
            }
            .fixedSize()
            .help("Try something else from here, while this session stays as it is")
            if store.ships {
                Menu {
                    Button("Land…") { store.send(.landTapped) }
                    Button("Update from the Base Branch") { store.send(.updateFromBaseTapped) }
                        .disabled(store.updating || store.isWorking)
                } label: {
                    Label("Ship", systemImage: "shippingbox")
                }
                .fixedSize()
                .help("Land the work on a branch or as pull requests, or merge the base branch's new commits in")
            }
            Menu {
                Button("Copy Folder Path") { copy(store.session.workingDirectory) }
                if store.session.host == nil {
                    Button("Show in Finder") {
                        NSWorkspace.shared.activateFileViewerSelecting([folder])
                    }
                    Button("Open in Terminal") { open(folder, with: "com.apple.Terminal") }
                }
            } label: {
                Label("Folder", systemImage: "folder")
            }
            .fixedSize()
            .help(
                store.session.host.map { "The folder it works in on \($0)" }
                    ?? "The folder it works in, to look at or work in yourself")
            Picker("Approvals", selection: approvalMode) {
                ForEach(ApprovalMode.allCases, id: \.self) { mode in
                    Text(Self.name(mode)).tag(mode)
                }
            }
            .pickerStyle(.menu)
            .fixedSize()
            .help("What the agent may do without asking")
        }
    }

    /// ⌘1, ⌘2 and so on for the views, in the order the picker shows them,
    /// and ⌘T for another terminal.
    private var tabShortcuts: some View {
        ZStack {
            if store.tabs.contains(.terminal) {
                Button("New Terminal") { store.send(.newTerminalTapped) }
                    .keyboardShortcut("t", modifiers: .command)
            }
            ForEach(Array(store.tabs.enumerated()), id: \.element) { index, tab in
                Button(title(tab)) { store.send(.tabPicked(tab)) }
                    .keyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: .command)
            }
        }
        .opacity(0)
        .accessibilityHidden(true)
    }

    private var folder: URL { URL(filePath: store.session.workingDirectory, directoryHint: .isDirectory) }

    private func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    private func open(_ folder: URL, with bundleID: String) {
        guard let app = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID) else { return }
        NSWorkspace.shared.open([folder], withApplicationAt: app, configuration: NSWorkspace.OpenConfiguration())
    }

    private var approvalMode: Binding<ApprovalMode> {
        Binding(get: { store.session.approvalMode }, set: { store.send(.approvalModePicked($0)) })
    }

    private var engine: String {
        let backend =
            switch store.session.backend {
            case .copilot: "Copilot"
            case .claude: "Claude Code"
            case .local: "Local model"
            case .scripted: "Scripted"
            }
        let model = store.session.model.map { " · \($0)" } ?? ""
        let host = store.session.host.map { " on \($0)" } ?? ""
        return backend + model + host
    }

    private var chat: some View {
        ScrollViewReader { reader in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    ForEach(store.items) { item in
                        row(item)
                            .padding(item.id == store.highlightedItem ? 6 : 0)
                            .background {
                                if item.id == store.highlightedItem {
                                    RoundedRectangle(cornerRadius: 8).fill(Color.yellow.opacity(0.18))
                                }
                            }
                            .id(item.id)
                    }
                    if store.isWorking, !store.isWaiting {
                        ReplyInProgress(
                            text: store.streaming, thinking: nil, showsThinking: false, toggleThinking: {})
                    }
                }
                .padding()
            }
            .defaultScrollAnchor(.bottom)
            .onChange(of: store.highlightedItem, initial: true) { _, item in
                guard let item else { return }
                reader.scrollTo(item, anchor: .center)
            }
        }
        .safeAreaInset(edge: .top) {
            if store.highlighted != nil {
                highlightBanner
            }
        }
    }

    /// Over the chat while a search hit is highlighted: forks from it, or
    /// goes back to the chat as it is.
    private var highlightBanner: some View {
        HStack {
            Image(systemName: "text.magnifyingglass")
                .foregroundStyle(.secondary)
            Text(
                store.highlightedItem == nil
                    ? "Finding what the search found…" : "What the search found is highlighted.")
            Spacer()
            Button("Fork from Here") { store.send(.forkFromHighlightTapped) }
                .disabled(store.highlightedItem == nil)
                .help("A new session with this chat up to the end of that turn, and the code as it was then.")
            Button("Done") { store.send(.highlightDismissed) }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.bar)
    }

    @ViewBuilder
    private func row(_ item: ChatItem) -> some View {
        switch item {
        case .person(let id, let text):
            if store.editing == id {
                editor
            } else {
                MessageView(role: .person, text: text, select: { selecting = TextToSelect(text: $0) })
                    .contextMenu {
                        Button("Edit Message…") { store.send(.editTapped(id)) }
                        Divider()
                        Button("Fork Before This With the Code") {
                            store.send(.forkTapped(before: id, chatOnly: false))
                        }
                        Button("Fork Before This, Chat Only") {
                            store.send(.forkTapped(before: id, chatOnly: true))
                        }
                    }
            }
        case .reply(_, let text):
            MessageView(role: .assistant, text: text, select: { selecting = TextToSelect(text: $0) })
        case .thinking(let id, let text):
            ThinkingDisclosure(
                thinking: ModelThinking(text: text), isExpanded: store.expandedThinking.contains(id),
                toggle: { store.send(.thinkingToggled(id: id)) }, select: { selecting = TextToSelect(text: $0) },
                answering: true)
        case .tool(let call):
            if let proposal = store.proposals.proposals[id: call.id] {
                ProposalCard(
                    proposal: proposal, deciding: store.proposals.deciding.contains(call.id),
                    canStartWork: store.session.planning
                ) { store.send(.proposals($0)) }
            } else {
                ToolCallView(call: call) { request, approved in
                    store.send(.answerPermission(requestID: request, approved: approved))
                }
            }
        case .permission(_, let permission):
            PermissionCard(permission: permission) { request, approved in
                store.send(.answerPermission(requestID: request, approved: approved))
            }
        case .cost(_, let usage):
            Label(Self.describe(usage), systemImage: "gauge.with.dots.needle.33percent")
                .font(.caption)
                .foregroundStyle(.tertiary)
                .frame(maxWidth: .infinity, alignment: .trailing)
                .help("What this turn used")
        case .stopped:
            Label("Stopped", systemImage: "stop.circle")
                .font(.footnote)
                .foregroundStyle(.secondary)
        case .failure(_, let message):
            Label(message, systemImage: "exclamationmark.triangle")
                .font(.footnote)
                .foregroundStyle(.red)
        }
    }

    /// The person's earlier message, open to change. Sending it forks, so
    /// this session stays as it was.
    private var editor: some View {
        VStack(alignment: .trailing, spacing: 8) {
            TextEditor(text: $store.edited)
                .font(.body)
                .frame(minHeight: 60)
                .scrollContentBackground(.hidden)
                .padding(6)
                .background(.fill.quaternary, in: .rect(cornerRadius: 10))
                .accessibilityLabel("Edited message")
            HStack {
                Text("Sending starts a fork from before this message; this session stays as it is.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Cancel") { store.send(.editCancelled) }
                Button("Send to a Fork") { store.send(.editSent) }
                    .buttonStyle(.borderedProminent)
                    .disabled(store.edited.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
    }

    private var composer: some View {
        ChatComposer(
            text: $store.draft, prompt: Text(store.isWorking ? "Queue a message, or steer" : "Message the agent"),
            model: nil,
            send: SendButton(
                canSend: !store.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                canStop: store.isWorking,
                send: { store.send(.sendTapped(.enqueue)) },
                stop: { store.send(.abortTapped) })
        ) {
            store.send(.sendTapped(.enqueue))
        } top: {
            ForEach(store.queued) { queued in
                QueuedRow(queued: queued) { store.send(.unqueueTapped(queued.id)) }
            }
            if store.isWorking, !store.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                HStack {
                    Text("Return queues it for after this turn.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("Steer Now") { store.send(.sendTapped(.steer)) }
                        .font(.caption.weight(.semibold))
                        .keyboardShortcut(.return, modifiers: .command)
                        .help("Send it to the running turn (⌘↩)")
                }
                .padding(.horizontal, 4)
            }
        }
    }

    static func name(_ mode: ApprovalMode) -> String {
        switch mode {
        case .askEveryTime: "Ask Every Time"
        case .allowEdits: "Allow Edits"
        case .allowEverything: "Allow Everything"
        }
    }

    static func describe(_ usage: Usage) -> String {
        let tokens =
            "\(usage.inputTokens.formatted(.number.notation(.compactName))) in · "
            + "\(usage.outputTokens.formatted(.number.notation(.compactName))) out"
        guard usage.premiumRequests > 0 else { return "\(usage.model) · \(tokens)" }
        return "\(usage.model) · \(tokens) · \(usage.premiumRequests.formatted()) premium"
    }
}

/// A tool the agent called: what it ran or wrote, the permission it asked
/// for, and its output, folded away until opened.
struct ToolCallView: View {
    let call: ToolCall
    let answer: (String, Bool) -> Void
    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                expanded.toggle()
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: icon)
                        .foregroundStyle(tint)
                    Text(call.subject ?? call.tool)
                        .font(.callout.monospaced())
                        .lineLimit(1)
                        .truncationMode(.middle)
                    if call.output == nil, call.permission?.answer != nil || call.permission == nil {
                        ProgressView().controlSize(.mini)
                    }
                    Spacer()
                    if call.output != nil || call.change != nil {
                        Image(systemName: "chevron.right")
                            .font(.caption.weight(.semibold))
                            .rotationEffect(.degrees(expanded ? 90 : 0))
                            .foregroundStyle(.secondary)
                    }
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .disabled(call.output == nil && call.change == nil)
            // A write waiting on the person shows what it would change.
            if let change = call.change, expanded || (call.permission != nil && call.permission?.answer == nil) {
                ChangeView(change: change)
            }
            if let permission = call.permission {
                PermissionCard(permission: permission, answer: answer)
            }
            if expanded, let output = call.output {
                Text(output)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
                    .background(.fill.quinary, in: .rect(cornerRadius: 8))
            }
        }
        .padding(10)
        .background(.fill.quaternary.opacity(0.5), in: .rect(cornerRadius: 12))
    }

    private var icon: String {
        switch call.succeeded {
        case true?: "checkmark.circle.fill"
        case false?: "xmark.circle.fill"
        case nil: call.tool == "shell" ? "terminal" : "doc.text"
        }
    }

    private var tint: Color {
        switch call.succeeded {
        case true?: .green
        case false?: .red
        case nil: .secondary
        }
    }
}

/// The lines a write takes out, in red, and puts in, in green.
struct ChangeView: View {
    let change: ToolCall.Change
    /// Lines past this are left out, with a note, to keep the chat quick.
    private static let shown = 300

    var body: some View {
        let lines = change.removed.map { ("-", $0) } + change.added.map { ("+", $0) }
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(lines.prefix(Self.shown).enumerated()), id: \.offset) { _, line in
                Text("\(line.0) \(line.1)")
                    .font(.caption.monospaced())
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 6)
                    .background(line.0 == "-" ? Color.red.opacity(0.15) : Color.green.opacity(0.15))
            }
            if lines.count > Self.shown {
                Text("\(lines.count - Self.shown) more lines")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(6)
            }
        }
        .textSelection(.enabled)
        .clipShape(.rect(cornerRadius: 8))
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(change.removed.count) lines removed, \(change.added.count) added")
    }
}

/// A request for permission: Approve and Deny while the agent waits, then
/// how it was answered.
struct PermissionCard: View {
    let permission: Permission
    let answer: (String, Bool) -> Void

    var body: some View {
        switch permission.answer {
        case nil:
            HStack(spacing: 8) {
                Label(permission.summary, systemImage: "hand.raised")
                    .font(.callout)
                    .lineLimit(2)
                Spacer()
                Button("Deny") { answer(permission.requestID, false) }
                Button("Approve") { answer(permission.requestID, true) }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
            }
            .padding(8)
            .background(.orange.opacity(0.12), in: .rect(cornerRadius: 8))
        case .approved?:
            answered("Approved", systemImage: "checkmark")
        case .denied?:
            answered("Denied", systemImage: "xmark")
        case .allowedByMode?:
            answered("Allowed by the approval mode", systemImage: "checkmark.shield")
        }
    }

    private func answered(_ text: LocalizedStringKey, systemImage: String) -> some View {
        Label(text, systemImage: systemImage)
            .font(.caption)
            .foregroundStyle(.secondary)
    }
}

/// Text opened to select part of.
private struct TextToSelect: Identifiable {
    let id = UUID()
    let text: String
}

/// A message waiting for its turn, above the box, with a way to take it out.
private struct QueuedRow: View {
    let queued: QueuedMessage
    let remove: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "clock")
                .foregroundStyle(.secondary)
            Text(queued.text)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer()
            Button("Remove", systemImage: "xmark.circle.fill", action: remove)
                .labelStyle(.iconOnly)
                .buttonStyle(.borderless)
                .foregroundStyle(.secondary)
                .help("Take it out before its turn starts")
        }
        .font(.caption)
        .padding(.horizontal, 6)
        .padding(.vertical, 4)
        .background(.fill.quaternary, in: .rect(cornerRadius: 6))
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Queued: \(queued.text)")
    }
}
