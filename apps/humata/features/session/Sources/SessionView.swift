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
import UniformTypeIdentifiers

/// A session's chat: the person's messages, the agent's thinking, replies,
/// the commands it runs with their output and the permissions it asks for,
/// with the field to send, steer or queue.
package struct SessionView: View {
    @Bindable var store: StoreOf<SessionFeature>
    @State private var selecting: TextToSelect?

    package init(store: StoreOf<SessionFeature>) {
        self.store = store
    }

    /// A reply's code into the message box, as a quote.
    private var quoter: CodeQuoter {
        CodeQuoter { [store] language, code in
            store.send(.quoted(CodeQuote.text(path: nil, start: nil, end: nil, language: language ?? "", code: code)))
        }
    }

    package var body: some View {
        VStack(spacing: 0) {
            header
                // The narrowest header fits; the window goes no narrower.
                .frame(minWidth: 560)
            Divider()
            if store.tab == .review {
                ReviewView(store: store.scope(state: \.review, action: \.review))
            } else if store.tab == .files {
                DocsView(store: store.scope(state: \.docs, action: \.docs))
            } else if store.tab == .spec {
                SpecsView(store: store.scope(state: \.specs, action: \.specs))
            } else if store.tab == .terminal {
                TerminalsView(store: store.scope(state: \.terminals, action: \.terminals))
            } else {
                chat
                    .alert($store.scope(state: \.proposals.alert, action: \.proposals.alert))
                    .environment(\.codeQuoter, store.session.archived ? nil : quoter)
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
                SessionComposer(store: store)
                    .padding([.horizontal, .bottom])
            }
        }
        .navigationTitle(store.session.name.isEmpty ? "Session" : store.session.name)
        .task { await store.send(.task).finish() }
        .alert($store.scope(state: \.alert, action: \.alert))
        .confirmationDialog($store.scope(state: \.rewindDialog, action: \.rewindDialog))
        .overlay(alignment: .top) {
            if store.rewinding {
                Label {
                    Text("Rewinding…")
                } icon: {
                    ProgressView().controlSize(.small)
                }
                .font(.callout)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .background(.regularMaterial, in: .capsule)
                .padding(.top, 8)
            }
        }
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
        case .spec: "Spec"
        case .terminal: "Terminal"
        }
    }

    /// The title, the tabs and the actions, giving up the usage, then the
    /// actions' names, then the tabs' row as the window narrows, so nothing
    /// is cut off.
    private var header: some View {
        ViewThatFits(in: .horizontal) {
            headerRow(usage: true, names: true, tabsInAMenu: false)
            headerRow(usage: false, names: true, tabsInAMenu: false)
            headerRow(usage: false, names: false, tabsInAMenu: false)
            headerRow(usage: false, names: false, tabsInAMenu: true)
        }
        .padding(.horizontal)
        .padding(.vertical, 10)
    }

    private func headerRow(usage showsUsage: Bool, names: Bool, tabsInAMenu: Bool) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(store.session.name.isEmpty ? "Session" : store.session.name)
                    .font(.headline)
                    .lineLimit(1)
                Text(engine)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                if let task = store.session.itemTitle {
                    Button {
                        store.send(.itemTapped)
                    } label: {
                        Label(task, systemImage: "checklist")
                            .font(.caption)
                            .lineLimit(1)
                    }
                    .buttonStyle(.link)
                    .help("Open \(task) on the board")
                }
            }
            // A long title is cut short rather than squeezing the rest out.
            .frame(minWidth: 90, maxWidth: 260, alignment: .leading)
            Spacer(minLength: 0)
            tabPicker(inAMenu: tabsInAMenu)
            Spacer(minLength: 0)
            if showsUsage, let usage = store.usage {
                Text(Self.describe(usage))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .help("What this session has used")
            }
            actions(names: names)
        }
    }

    @ViewBuilder
    private func tabPicker(inAMenu: Bool) -> some View {
        let picker = Picker("View", selection: Binding(get: { store.tab }, set: { store.send(.tabPicked($0)) })) {
            ForEach(store.tabs, id: \.self) { tab in
                Text(title(tab)).tag(tab)
            }
        }
        .labelsHidden()
        .fixedSize()
        .help(
            "The chat, the session's changes to review, the files in its folder, the specs it reads, "
                + "or a terminal there (⌘1 to ⌘5)"
        )
        .background { tabShortcuts }
        if inAMenu {
            picker.pickerStyle(.menu)
        } else {
            picker.pickerStyle(.segmented)
        }
    }

    /// Fork, Ship, Folder and the approval mode; without their names, and
    /// the approval mode without its label, when the window is too narrow.
    private func actions(names: Bool) -> some View {
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
                ForEach(ApprovalMode.offered(for: store.session.backend), id: \.self) { mode in
                    Text(Self.name(mode)).tag(mode)
                }
            }
            .pickerStyle(.menu)
            .modifier(HidesLabel(hidden: !names))
            .fixedSize()
            .help("What the agent may do without asking")
            if store.session.backend == .copilot {
                Toggle(
                    "Experimental", systemImage: "flask",
                    isOn: Binding(get: { store.session.experimental }, set: { store.send(.experimentalToggled($0)) })
                )
                .toggleStyle(.button)
                .fixedSize()
                .help(
                    "Copilot's features still being tried out, from the next message. Fusion needs them to pick "
                        + "local models.")
            }
        }
        .modifier(IconsOnly(enabled: !names))
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
                    TurnInProgress(store: store)
                }
                .padding()
            }
            .defaultScrollAnchor(.bottom)
            .overlay {
                if !store.historyLoaded, store.items.isEmpty {
                    ProgressView("Loading the chat…")
                        .controlSize(.small)
                        .accessibilityIdentifier("chat.loading")
                }
            }
            .onChange(of: store.highlightedItem, initial: true) { _, item in
                guard let item else { return }
                reader.scrollTo(item, anchor: .center)
            }
            .onChange(of: store.scrollTarget) { _, item in
                guard let item else { return }
                withAnimation { reader.scrollTo(item, anchor: .top) }
                store.send(.scrolled)
            }
        }
        .safeAreaInset(edge: .top) {
            if store.highlighted != nil {
                highlightBanner
            }
        }
        // An inset, not a row over the box: in the stack beside the chat it
        // changed the chat's frame, and the chat, kept at its bottom, laid
        // itself out again without end.
        .safeAreaInset(edge: .bottom) {
            WaitingStrip(store: store)
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
        case .person(let id, let text, let images):
            if store.editing == id {
                editor
            } else {
                MessageView(
                    role: .person, text: text, select: { selecting = TextToSelect(text: $0) },
                    attachments: {
                        if !images.isEmpty {
                            SentImages(names: images, thumbnails: store.thumbnails) { store.send(.thumbnailNeeded($0)) }
                        }
                    }
                )
                .contextMenu {
                    Button("Edit Message…") { store.send(.editTapped(id)) }
                    Button("Rewind to Before This…") { store.send(.rewindTapped(id)) }
                        .disabled(store.rewinding || store.session.archived)
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
                ToolCallView(call: call, select: { selecting = TextToSelect(text: $0) }) { request, approved in
                    store.send(.answerPermission(requestID: request, approved: approved))
                }
                if let failure = store.proposals.listFailure, store.state.proposes(call.id) {
                    Label("Couldn’t read this proposal: \(failure)", systemImage: "exclamationmark.triangle")
                        .font(.caption)
                        .foregroundStyle(.secondary)
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
        case .rewound(_, let files, let undoable):
            HStack(alignment: .firstTextBaseline) {
                Label(
                    files ? "Rewound the chat and files to here" : "Rewound the chat to here",
                    systemImage: "arrow.uturn.backward.circle"
                )
                .font(.footnote)
                .foregroundStyle(.secondary)
                if undoable {
                    Spacer()
                    Button("Undo") { store.send(.undoRewindTapped) }
                        .disabled(store.rewinding)
                        .help("Bring back what the rewind cut; possible until you send another message")
                }
            }
        case .compacted:
            Label(
                "Context compacted: the agent summed up what came before to make room",
                systemImage: "rectangle.compress.vertical"
            )
            .font(.footnote)
            .foregroundStyle(.secondary)
        case .notice(_, let text):
            Label(text, systemImage: "info.circle")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
        case .failure(let id, let message):
            HStack(alignment: .firstTextBaseline) {
                Label(message, systemImage: "exclamationmark.triangle")
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .textSelection(.enabled)
                if id == store.retryableFailure, !store.isWorking {
                    Spacer()
                    Button("Try Again") { store.send(.retryTapped) }
                        .help("Send your last message again")
                }
            }
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

    static func name(_ mode: ApprovalMode) -> String {
        switch mode {
        case .askEveryTime: "Ask Every Time"
        case .allowEdits: "Allow Edits"
        case .allowEverything: "Allow Everything"
        case .auto: "Auto"
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
    /// Opens text to select from, for output too long to show in the chat.
    var select: (String) -> Void = { _ in }
    let answer: (String, Bool) -> Void
    @State private var expanded = false

    /// Output longer than this many lines scrolls in a box of its own, so
    /// opening and closing it doesn't lay the whole chat out again.
    private static let boxedLines = 24
    /// Characters of output shown in the chat; the rest is a click away.
    private static let shownCharacters = 20_000

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
                    .simultaneousGesture(collapse)
            }
            if let permission = call.permission {
                PermissionCard(permission: permission, answer: answer)
            }
            if expanded, let output = call.output {
                outputView(output)
                    .simultaneousGesture(collapse)
            }
        }
        .padding(10)
        .background(.fill.quaternary.opacity(0.5), in: .rect(cornerRadius: 12))
    }

    /// A double-click on what's open closes it, wherever it was scrolled
    /// to; dragging still selects.
    private var collapse: some Gesture {
        TapGesture(count: 2).onEnded { expanded = false }
    }

    @ViewBuilder
    private func outputView(_ output: String) -> some View {
        let cut = output.count > Self.shownCharacters
        let shown = cut ? String(output.prefix(Self.shownCharacters)) : output
        let text = Text(shown)
            .font(.caption.monospaced())
            .foregroundStyle(.secondary)
            .textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(8)
        VStack(alignment: .leading, spacing: 4) {
            if shown.utf8.lazy.filter({ $0 == 10 }).count > Self.boxedLines {
                ScrollView { text }
                    .frame(height: 320)
                    .background(.fill.quinary, in: .rect(cornerRadius: 8))
            } else {
                text.background(.fill.quinary, in: .rect(cornerRadius: 8))
            }
            if cut {
                HStack {
                    Text("The first \(Self.shownCharacters.formatted()) of \(output.count.formatted()) characters")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Button("Show All") { select(output) }
                    .help("Open all of it to read and select; double-click the output to close it")
                        .buttonStyle(.link)
                        .font(.caption)
                }
            }
        }
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
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: "hand.raised")
                FullCommand(text: permission.summary)
                Button("Deny") { answer(permission.requestID, false) }
                // No Return shortcut: Return in the composer sends the
                // message, and must never approve. ⇧⌘A and ⇧⌘D answer.
                Button("Approve") { answer(permission.requestID, true) }
                    .buttonStyle(.borderedProminent)
            }
            .padding(8)
            .background(.orange.opacity(0.12), in: .rect(cornerRadius: 8))
        case .approved?:
            answered("Approved", systemImage: "checkmark")
        case .denied?:
            answered("Denied", systemImage: "xmark")
        case .allowedByMode?:
            answered("Allowed by the approval mode", systemImage: "checkmark.shield")
        case .allowedByJudge?:
            answered("Allowed by the agent’s judge, in Auto", systemImage: "checkmark.seal")
        case .notAnswered?:
            answered("Not answered; the turn ended first", systemImage: "clock.badge.xmark")
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

/// A message waiting for its turn, above the box, with ways to edit it in
/// place and to take it out. What's sent while it waits joins it.
private struct QueuedRow: View {
    let queued: QueuedMessage
    let edit: () -> Void
    let remove: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "clock")
                .foregroundStyle(.secondary)
            Text(queued.text)
                .lineLimit(1)
                .truncationMode(.tail)
            if !queued.images.isEmpty {
                Label("\(queued.images.count)", systemImage: "photo")
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                    .help(
                        queued.images.count == 1 ? "1 image goes with it" : "\(queued.images.count) images go with it")
            }
            Spacer()
            Button("Edit", systemImage: "pencil", action: edit)
                .labelStyle(.iconOnly)
                .buttonStyle(.borderless)
                .foregroundStyle(.secondary)
                .help("Change it before its turn starts")
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

/// The images sent with a message, as thumbnails, each read when it first
/// shows.
/// Thumbnails decoded once each, not on every draw of the chat.
@MainActor
private enum Thumbnails {
    private static let cache = NSCache<NSData, NSImage>()

    static func image(_ data: Data) -> NSImage? {
        if let image = cache.object(forKey: data as NSData) { return image }
        guard let image = NSImage(data: data) else { return nil }
        cache.setObject(image, forKey: data as NSData)
        return image
    }
}

private struct SentImages: View {
    let names: [String]
    let thumbnails: [String: Data]
    let needed: (String) -> Void

    var body: some View {
        HStack(spacing: 6) {
            ForEach(names, id: \.self) { name in
                Group {
                    if let data = thumbnails[name] {
                        if let image = Thumbnails.image(data) {
                            Image(nsImage: image)
                                .resizable()
                                .scaledToFill()
                        } else {
                            Image(systemName: "photo.badge.exclamationmark")
                                .foregroundStyle(.secondary)
                                .help("This image is gone from the session")
                        }
                    } else {
                        ProgressView().controlSize(.small)
                    }
                }
                .frame(width: 96, height: 96)
                .background(.fill.quaternary)
                .clipShape(.rect(cornerRadius: 8))
                .onAppear { needed(name) }
                .accessibilityLabel("Image")
            }
        }
    }
}

/// The images in the box: a spinner on each while it's made ready to
/// send, then its thumbnail, each with a way to take it out.
private struct ImageChips: View {
    let images: IdentifiedArrayOf<DraftImage>
    let remove: (String) -> Void

    var body: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 6) {
                ForEach(images) { image in
                    ZStack(alignment: .topTrailing) {
                        Group {
                            if image.isPreparing {
                                ProgressView().controlSize(.small)
                                    .help("Getting \(image.name) ready to send")
                            } else if let data = image.thumbnail, let thumbnail = Thumbnails.image(data) {
                                Image(nsImage: thumbnail)
                                    .resizable()
                                    .scaledToFill()
                            } else {
                                Image(systemName: "photo")
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .frame(width: 52, height: 52)
                        .background(.fill.quaternary)
                        .clipShape(.rect(cornerRadius: 8))
                        Button("Remove", systemImage: "xmark.circle.fill") { remove(image.id) }
                            .labelStyle(.iconOnly)
                            .buttonStyle(.borderless)
                            .foregroundStyle(.secondary)
                            .background(.background, in: .circle)
                            .offset(x: 4, y: -4)
                            .help("Take it out of the message")
                    }
                    .padding(.top, 4)
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(image.isPreparing ? "\(image.name), getting ready" : image.name)
                }
            }
        }
        .scrollIndicators(.never)
    }
}

/// The long texts pasted into the box, each a chip to read or take out.
private struct PasteChips: View {
    let pastes: [Paste]
    let remove: (String) -> Void
    @State private var open: OpenAttachment?

    var body: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 6) {
                ForEach(pastes) { paste in
                    HStack(spacing: 4) {
                        Button {
                            open = .file(name: "Pasted Text", text: paste.text)
                        } label: {
                            Label(paste.label, systemImage: "doc.on.clipboard")
                                .monospacedDigit()
                        }
                        .buttonStyle(.borderless)
                        .help("Read what was pasted")
                        Button("Remove", systemImage: "xmark.circle.fill") { remove(paste.id) }
                            .labelStyle(.iconOnly)
                            .buttonStyle(.borderless)
                            .foregroundStyle(.secondary)
                            .help("Take it out of the message")
                    }
                    .font(.caption)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(.fill.quaternary, in: .capsule)
                }
            }
        }
        .scrollIndicators(.never)
        .attachmentViewer($open)
    }
}

/// A queued message open to edit, until it's saved or cancelled; Return
/// saves, and Escape cancels.
private struct QueuedEditor: View {
    @Binding var text: String
    let save: () -> Void
    let cancel: () -> Void
    @FocusState private var focused: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Image(systemName: "clock")
                .foregroundStyle(.secondary)
            TextField("Queued message", text: $text, axis: .vertical)
                .textFieldStyle(.plain)
                .lineLimit(1...6)
                .focused($focused)
                .onSubmit(save)
                .onKeyPress(.escape) {
                    cancel()
                    return .handled
                }
            Button("Cancel", action: cancel)
                .buttonStyle(.borderless)
            Button("Save", action: save)
                .buttonStyle(.borderless)
                .fontWeight(.semibold)
        }
        .font(.caption)
        .padding(.horizontal, 6)
        .padding(.vertical, 4)
        .background(.fill.tertiary, in: .rect(cornerRadius: 6))
        .onAppear { focused = true }
    }
}

/// A queued message that never ran because Humata stopped before its turn,
/// with a way to put it back in the draft; it isn't sent again unasked.
private struct DroppedRow: View {
    let message: QueuedMessage
    let putBack: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "exclamationmark.circle")
                .foregroundStyle(.orange)
            Text(message.text)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer()
            Button("Put in Draft", action: putBack)
                .buttonStyle(.borderless)
                .help("Humata stopped before this queued message's turn, so it didn't run")
        }
        .font(.caption)
        .padding(.horizontal, 6)
        .padding(.vertical, 4)
        .background(.fill.quaternary, in: .rect(cornerRadius: 6))
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Didn't run: \(message.text)")
    }
}

/// What a request for permission would run or write, whole: monospaced,
/// selectable, and scrolling past about twelve lines, so it can be read
/// before it's allowed.
struct FullCommand: View {
    let text: String

    var body: some View {
        ScrollView {
            Text(text)
                .font(.callout.monospaced())
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxHeight: 12 * 17)
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// Over the box, what the session waits on you for, so it isn't missed up
/// the chat: its question, the move it asks for, the first permission it
/// waits on, and the proposals not decided yet.
private struct WaitingStrip: View {
    let store: StoreOf<SessionFeature>

    var body: some View {
        let permissions = store.pendingPermissions
        let proposals = store.pendingProposals.count
        if store.request != nil || !permissions.isEmpty || proposals > 0 {
            VStack(alignment: .leading, spacing: 8) {
                if let request = store.request {
                    requestRow(request)
                }
                if let first = permissions.first,
                    case .permissionRequested(let id, let kind, let summary) = first.kind
                {
                    permissionRow(id: id, kind: kind, summary: summary, more: permissions.count - 1)
                }
                if proposals > 0 {
                    HStack {
                        Label(
                            proposals == 1 ? "1 proposal waits for you" : "\(proposals) proposals wait for you",
                            systemImage: "checklist")
                        Spacer()
                        Button("Show") { store.send(.showProposalsTapped) }
                            .help("Scroll to the first proposal not decided yet")
                    }
                }
            }
            .font(.callout)
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.orange.opacity(0.1), in: .rect(cornerRadius: 10))
            .padding(.horizontal)
            .padding(.bottom, 6)
        }
    }

    @ViewBuilder
    private func requestRow(_ request: AgentRequest) -> some View {
        switch request.kind {
        case .question:
            VStack(alignment: .leading, spacing: 2) {
                Label(LocalizedStringKey(request.text), systemImage: "questionmark.bubble")
                    .textSelection(.enabled)
                Text("Your next message answers it.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        case .move(_, let status):
            HStack(alignment: .firstTextBaseline) {
                Label(LocalizedStringKey(request.text), systemImage: "checkmark.seal")
                    .textSelection(.enabled)
                Spacer()
                Button("Open Changes") { store.send(.tabPicked(.review)) }
                Button("Move to \(status.title)") { store.send(.moveApproved) }
                    .buttonStyle(.borderedProminent)
            }
        }
    }

    private func permissionRow(id: String, kind: PermissionKind, summary: String, more: Int) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .top) {
                Image(systemName: "hand.raised")
                // A few lines only: the card in the chat shows it whole. A
                // scroll view here, under a chat kept at its bottom, makes
                // the two lay each other out without end.
                Text(summary)
                    .font(.callout.monospaced())
                    .lineLimit(3)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button("Deny") { store.send(.answerPermission(requestID: id, approved: false)) }
                    .keyboardShortcut("d", modifiers: [.command, .shift])
                    .help("Deny (⇧⌘D)")
                Button("Approve") { store.send(.answerPermission(requestID: id, approved: true)) }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut("a", modifiers: [.command, .shift])
                    .help("Approve (⇧⌘A)")
                Menu {
                    let widened: ApprovalMode = kind == .write ? .allowEdits : .allowEverything
                    Button(kind == .write ? "Allow Edits from Now On" : "Allow Everything from Now On") {
                        store.send(.approvalModePicked(widened))
                    }
                } label: {
                    Image(systemName: "chevron.down")
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                .help("Stop asking in this session")
            }
            if more > 0 {
                Text("and \(more) more")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

/// The turn under way at the chat's end: why it waits, or the reply as it
/// streams. Its own view, so each streamed word redraws only this and not
/// every row of the chat.
private struct TurnInProgress: View {
    let store: StoreOf<SessionFeature>

    var body: some View {
        if store.isWorking, let reason = store.waitReason {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                ProgressView().controlSize(.small)
                Text(reason).foregroundStyle(.secondary)
            }
        } else if store.isWorking, !store.isWaiting {
            ReplyInProgress(text: store.streaming, thinking: nil, showsThinking: false, toggleThinking: {})
        }
    }
}

/// The box to write in, with the messages queued over it. Its own view, so
/// each key typed redraws only this and not the chat.
private struct SessionComposer: View {
    @Bindable var store: StoreOf<SessionFeature>
    @State private var choosing = false

    var body: some View {
        composer
            .fileImporter(isPresented: $choosing, allowedContentTypes: [.image], allowsMultipleSelection: true) {
                if case .success(let urls) = $0 { store.send(.imagesAdded(urls)) }
            }
            .onDrop(of: [.fileURL, .image], isTargeted: nil) { providers in
                // One the model can't see is refused with a note saying so.
                guard !store.session.archived else { return false }
                return drop(providers)
            }
    }

    /// Adds dropped image files, and images dragged from other apps.
    private func drop(_ providers: [NSItemProvider]) -> Bool {
        var taken = false
        for provider in providers {
            if provider.canLoadObject(ofClass: URL.self) {
                taken = true
                _ = provider.loadObject(ofClass: URL.self) { url, _ in
                    guard let url, UTType(filenameExtension: url.pathExtension)?.conforms(to: .image) == true else {
                        return
                    }
                    Task { @MainActor in store.send(.imagesAdded([url])) }
                }
            } else if provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
                taken = true
                provider.loadDataRepresentation(forTypeIdentifier: UTType.image.identifier) { data, _ in
                    guard let data else { return }
                    Task { @MainActor in store.send(.imagePasted(data)) }
                }
            }
        }
        return taken
    }

    /// The image on the clipboard, if there is one. Read only when asked
    /// for, as it can be large.
    private var pastedImage: Data? {
        let pasteboard = NSPasteboard.general
        for type in [NSPasteboard.PasteboardType.png, .tiff] {
            if let data = pasteboard.data(forType: type) { return data }
        }
        return nil
    }

    private var prompt: Text { Text(store.isWorking ? "Queue a message, or steer" : "Message the agent") }

    private var sendButton: SendButton {
        SendButton(
            canSend: store.canSend,
            canStop: store.isWorking,
            send: { store.send(.sendTapped(.enqueue)) },
            stop: { store.send(.abortTapped) })
    }

    /// With + for images only when the model can see them.
    @ViewBuilder private var composer: some View {
        if store.takesImages {
            ChatComposer(text: $store.draft, prompt: prompt, model: nil, send: sendButton) {
                store.send(.sendTapped(.enqueue))
            } top: {
                top
            } attach: {
                Button("Choose Images…", systemImage: "photo") { choosing = true }
                // Always on: the clipboard can change while the menu is
                // closed, so it's read only when picked.
                Button("Paste Image", systemImage: "doc.on.clipboard") {
                    if let data = pastedImage {
                        store.send(.imagePasted(data))
                    } else {
                        store.send(.noImageToPaste)
                    }
                }
            }
        } else {
            ChatComposer(text: $store.draft, prompt: prompt, model: nil, send: sendButton) {
                store.send(.sendTapped(.enqueue))
            } top: {
                top
            }
        }
    }

    /// What sits over the field: the message's images and pastes, and the
    /// messages queued.
    @ViewBuilder private var top: some View {
        if !store.images.isEmpty {
            ImageChips(images: store.images) { store.send(.removeImageTapped($0)) }
        }
        if !store.pastes.isEmpty {
            PasteChips(pastes: store.pastes) { store.send(.removePasteTapped($0)) }
        }
        ForEach(store.dropped) { dropped in
            DroppedRow(message: dropped) { store.send(.putBackTapped(dropped.id)) }
        }
        ForEach(store.queued) { queued in
            if store.editingQueued == queued.id {
                QueuedEditor(
                    text: $store.queuedEdit, save: { store.send(.editQueuedSaved) },
                    cancel: { store.send(.editQueuedCancelled) })
            } else {
                QueuedRow(
                    queued: queued, edit: { store.send(.editQueuedTapped(queued.id)) },
                    remove: { store.send(.unqueueTapped(queued.id)) })
            }
        }
        if store.isWorking, store.draft.isEmpty, store.lastMessage != nil {
            HStack {
                Spacer()
                Button("Stop and Edit") { store.send(.stopAndEditTapped) }
                    .font(.caption)
                    .buttonStyle(.borderless)
                    .help("Stop this turn and put your message back in the box")
            }
            .padding(.horizontal, 4)
        }
        if store.isWorking, !store.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            HStack {
                Text(
                    store.queued.isEmpty
                        ? "Return queues it for after this turn."
                        : "Return adds it to what’s queued."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                Spacer()
                if store.images.isEmpty {
                    Button("Steer Now") { store.send(.sendTapped(.steer)) }
                        .font(.caption.weight(.semibold))
                        .keyboardShortcut(.return, modifiers: .command)
                        .help("Send it to the running turn (⌘↩)")
                }
            }
            .padding(.horizontal, 4)
        }
    }
}

/// Hides a control's label when `hidden`.
private struct HidesLabel: ViewModifier {
    let hidden: Bool

    func body(content: Content) -> some View {
        if hidden { content.labelsHidden() } else { content }
    }
}

/// Labels as icons alone when `enabled`, otherwise with their titles.
private struct IconsOnly: ViewModifier {
    let enabled: Bool

    func body(content: Content) -> some View {
        if enabled { content.labelStyle(.iconOnly) } else { content.labelStyle(.titleAndIcon) }
    }
}
