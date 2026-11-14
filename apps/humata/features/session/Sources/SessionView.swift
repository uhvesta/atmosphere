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
            // In a view of its own, so what it reads, such as the usage,
            // redraws only it.
            Separately { header }
                // The narrowest header fits; the window goes no narrower.
                .frame(minWidth: 560)
            Divider()
            if store.tab == .review {
                ReviewView(store: store.scope(state: \.review, action: \.review))
            } else if store.tab == .files {
                if store.docs.focused {
                    focus(DocsView(store: store.scope(state: \.docs, action: \.docs)))
                } else {
                    DocsView(store: store.scope(state: \.docs, action: \.docs))
                }
            } else if store.tab == .spec {
                if store.specs.focused != nil {
                    focus(SpecsView(store: store.scope(state: \.specs, action: \.specs)))
                } else {
                    SpecsView(store: store.scope(state: \.specs, action: \.specs))
                }
            } else if store.tab == .terminal {
                TerminalsView(store: store.scope(state: \.terminals, action: \.terminals))
            } else {
                Fills { ChatList(store: store) }
                    .alert($store.scope(state: \.proposals.alert, action: \.proposals.alert))
                    .environment(\.codeQuoter, store.session.archived ? nil : quoter)
            }
            if store.disconnected, !store.session.hostOffline {
                ChatNotice(
                    store.watchFailure.map { LocalizedStringKey("Stopped watching the session: \($0) Reconnecting…") }
                        ?? "Lost the connection to Humata's daemon. Reconnecting…",
                    systemImage: "bolt.horizontal.circle",
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
            if let progress {
                Label {
                    Text(progress)
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
        .sheet(isPresented: editingProposal) {
            ProposalEditor(store: store.scope(state: \.proposals, action: \.proposals))
        }
    }

    /// What's under way that the person waits on, shown over the view.
    private var progress: LocalizedStringKey? {
        if store.rewinding { return "Rewinding…" }
        if store.forking { return "Forking…" }
        if store.updating { return "Updating from the base branch…" }
        return nil
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
            .disabled(store.forking)
            .fixedSize()
            .help("Try something else from here, while this session stays as it is")
            if store.ships {
                Menu {
                    if store.isWorking {
                        // Says why the two below are greyed out.
                        Text("Wait for the turn to end, or stop it, to land or update")
                    }
                    Button("Land…") { store.send(.landTapped) }
                        .disabled(store.isWorking)
                        .help(
                            store.isWorking
                                ? "The agent is still working; land once its turn ends, or stop it first"
                                : "Commit the work onto a new branch, and push it or open pull requests")
                    Button("Update from the Base Branch") { store.send(.updateFromBaseTapped) }
                        .disabled(store.updating || store.isWorking)
                        .help(
                            store.isWorking
                                ? "The agent is still working; merging the base in now would mix with its edits"
                                : store.updating
                                    ? "Already merging the base branch in"
                                    : "Merge the base branch's new commits into the session's copy")
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

    /// A document wide, with the chat and its composer beside it, so what's
    /// said about it goes into this chat.
    private func focus(_ document: some View) -> some View {
        HSplitView {
            document
                .frame(minWidth: 420, maxWidth: .infinity, maxHeight: .infinity)
            VStack(spacing: 0) {
                Fills { ChatList(store: store) }
                    .alert($store.scope(state: \.proposals.alert, action: \.proposals.alert))
                    .environment(\.codeQuoter, store.session.archived ? nil : quoter)
                if !store.session.archived {
                    SessionComposer(store: store)
                        .padding([.horizontal, .bottom])
                }
            }
            .frame(minWidth: 320, idealWidth: 400, maxWidth: 560, maxHeight: .infinity)
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

/// Takes all the space it's offered, whatever its view's size, and gives
/// it to the view: a chat growing as a reply is written lays out only
/// itself, not the header, composer and window around it.
private struct Fills: Layout {
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        proposal.replacingUnspecifiedDimensions(by: .zero)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for view in subviews {
            view.place(at: bounds.origin, proposal: ProposedViewSize(bounds.size))
        }
    }

    // Without these, a stack asking where this view's alignment guides are
    // has it place its view to find out, laying out all of it.
    func explicitAlignment(
        of guide: HorizontalAlignment, in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews,
        cache: inout ()
    ) -> CGFloat? { nil }

    func explicitAlignment(
        of guide: VerticalAlignment, in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews,
        cache: inout ()
    ) -> CGFloat? { nil }
}

/// The end of the chat, kept in view as it grows.
private enum ChatEnd {
    static func inView(_ geometry: ScrollGeometry) -> Bool {
        geometry.visibleRect.maxY >= geometry.contentSize.height - 40
    }

    /// Where the chat is scrolled, and how long it is.
    struct Place: Equatable {
        let offset: CGFloat
        let height: CGFloat
        let endInView: Bool

        init(geometry: ScrollGeometry) {
            offset = geometry.contentOffset.y
            height = geometry.contentSize.height
            endInView = ChatEnd.inView(geometry)
        }
    }
}

/// Draws `content` in a body of its own, so what it reads of the store
/// redraws only it and not the view around it.
private struct Separately<Content: View>: View {
    @ViewBuilder let content: () -> Content

    var body: some View { content() }
}

/// The chat's rows, with the turn under way at the end. Its own view, so
/// a change to the chat redraws only this; and each row is drawn again only
/// when what it shows changes.
private struct ChatList: View {
    let store: StoreOf<SessionFeature>
    @State private var selecting: TextToSelect?
    @State private var replyParts = ReplyParts()
    @State private var heights = RowHeights()
    /// Whether the chat keeps to its end as it grows: on while the end is in
    /// view, off once the person scrolls up to read.
    @State private var following = true
    /// Moves the chat to its end. Not `scrollTo` by id: that looks for the
    /// id past every row before it, and ran on each frame of a reply.
    @State private var position = ScrollPosition(idType: String.self)

    var body: some View {
        let highlighted = store.highlightedItem
        let select: (String) -> Void = { selecting = TextToSelect(text: $0) }
        ScrollViewReader { reader in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    ForEach(replyParts.lines(of: store.items, highlighted: highlighted)) { line in
                        let row = ChatRow(
                            item: line.item, part: line.part, shows: shows(line.item, highlighted: highlighted),
                            store: store, select: select
                        )
                        KeptHeight(id: line.id, row: row, heights: heights) {
                            row
                                .equatable()
                                // A reply's parts sit as close as its blocks do.
                                .padding(.top, line.continues ? -4 : 0)
                        }
                        .id(line.id)
                    }
                    TurnInProgress(store: store)
                }
                .padding()
            }
            .scrollPosition($position)
            // At its end when it opens; as it grows, what's in view stays.
            .defaultScrollAnchor(.bottom, for: .initialOffset)
            .onScrollGeometryChange(for: ChatEnd.Place.self) {
                ChatEnd.Place(geometry: $0)
            } action: { old, new in
                if new.height != old.height {
                    // Grown or shrunk with its end in view: keep to the end.
                    if old.endInView { following = true }
                    if following { position.scrollTo(edge: .bottom) }
                } else if new.offset < old.offset - 1, !new.endInView {
                    // Read up, by any means: a mouse wheel has no phases. An
                    // upward move that leaves the end in view is the chat
                    // getting shorter, as a row's guessed height is measured.
                    following = false
                } else if new.endInView, new.offset > old.offset {
                    following = true
                }
            }
            // Only the person's scrolling stops or starts the following:
            // rows measured as they come move what's in view by themselves.
            .onScrollPhaseChange { _, phase, context in
                switch phase {
                case .tracking, .interacting:
                    following = false
                case .idle:
                    // Only on: the stop after the chat's own scroll to its
                    // end can come while rows are still being measured.
                    if ChatEnd.inView(context.geometry) { following = true }
                case .decelerating, .animating:
                    break
                @unknown default:
                    break
                }
            }
            .overlay {
                if !store.historyLoaded, store.items.isEmpty {
                    if let fraction = store.historyLoading?.fraction {
                        ProgressView("Opening the chat…", value: fraction)
                            .frame(maxWidth: 240)
                            .accessibilityIdentifier("chat.loading")
                    } else {
                        ProgressView("Loading the chat…")
                            .controlSize(.small)
                            .accessibilityIdentifier("chat.loading")
                    }
                }
            }
            .onChange(of: highlighted, initial: true) { _, item in
                guard let item else { return }
                following = false
                reader.scrollTo(item, anchor: .center)
            }
            // A message sent takes the chat to its end to show it and what
            // the agent says to it, wherever the person had scrolled.
            .onChange(of: store.sending) { old, new in
                guard new > old else { return }
                following = true
                position.scrollTo(edge: .bottom)
            }
            .onChange(of: store.scrollTarget) { _, item in
                guard let item else { return }
                following = false
                withAnimation { reader.scrollTo(item, anchor: .top) }
                store.send(.scrolled)
            }
        }
        .safeAreaInset(edge: .top) {
            if store.highlighted != nil {
                highlightBanner(found: highlighted != nil)
            }
        }
        // An inset, not a row over the box: in the stack beside the chat it
        // changed the chat's frame, and the chat, kept at its bottom, laid
        // itself out again without end.
        .safeAreaInset(edge: .bottom) {
            WaitingStrip(store: store)
        }
        .sheet(item: $selecting) { SelectTextSheet(text: $0.text) }
    }

    /// What `item`'s row shows of the session besides the item, reading
    /// only what that kind of row needs.
    private func shows(_ item: ChatItem, highlighted: String?) -> ChatRow.Shows {
        var shows = ChatRow.Shows(highlighted: item.id == highlighted)
        switch item {
        case .person(let id, _, let images):
            shows.editing = store.editing == id
            shows.rewinding = store.rewinding
            shows.archived = store.session.archived
            for name in images { shows.thumbnails[name] = store.thumbnails[name] }
        case .thinking(let id, _):
            shows.thinkingOpen = store.expandedThinking.contains(id)
        case .tool(let call):
            shows.toolOpen = store.expandedTools.contains(call.id)
            if let proposal = store.proposals.proposals[id: call.id] {
                shows.proposal = proposal
                shows.deciding = store.proposals.deciding.contains(call.id)
                shows.canStartWork = store.session.planning
            } else if let failure = store.proposals.listFailure, store.proposingCalls.contains(call.id) {
                shows.proposalFailure = failure
            }
        case .rewound:
            shows.rewinding = store.rewinding
        case .failure(let id, _):
            shows.retryable = id == store.retryableFailure && !store.isWorking
        case .reply, .permission, .stopped, .notice, .cost, .compacted:
            break
        }
        return shows
    }

    /// Over the chat while a search hit is highlighted: forks from it, or
    /// goes back to the chat as it is.
    private func highlightBanner(found: Bool) -> some View {
        HStack {
            Image(systemName: "text.magnifyingglass")
                .foregroundStyle(.secondary)
            Text(found ? "What the search found is highlighted." : "Finding what the search found…")
            Spacer()
            Button("Fork from Here") { store.send(.forkFromHighlightTapped) }
                .disabled(!found)
                .help("A new session with this chat up to the end of that turn, and the code as it was then.")
            Button("Done") { store.send(.highlightDismissed) }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.bar)
    }
}

/// The heights chat rows had, each with what the row showed and the width
/// it had: a row scrolled back to, unchanged, isn't measured again. Without
/// them the stack measured each row it passed, and its guesses at heights
/// it hadn't measured were what made scrolling stall.
private final class RowHeights {
    private var kept: [String: (row: ChatRow, width: CGFloat, height: CGFloat)] = [:]

    func height(of id: String, row: ChatRow, width: CGFloat) -> CGFloat? {
        guard let kept = kept[id], kept.width == width, kept.row == row else { return nil }
        return kept.height
    }

    func keep(_ height: CGFloat, of id: String, row: ChatRow, width: CGFloat) {
        // Enough for a very long chat; past it, rows are measured again.
        if kept.count >= Self.limit { kept.removeAll(keepingCapacity: true) }
        kept[id] = (row, width, height)
    }

    private static let limit = 20_000
}

/// Lays out its one view as it is, but gives its height from `heights`
/// while the row and width are those it was measured with.
private struct KeptHeight: Layout {
    let id: String
    let row: ChatRow
    let heights: RowHeights

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let view = subviews.first else { return .zero }
        guard let width = proposal.width else { return view.sizeThatFits(proposal) }
        if let height = heights.height(of: id, row: row, width: width) {
            return CGSize(width: width, height: height)
        }
        let size = view.sizeThatFits(ProposedViewSize(width: width, height: nil))
        heights.keep(size.height, of: id, row: row, width: width)
        return CGSize(width: width, height: size.height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews.first?.place(
            at: bounds.origin, proposal: ProposedViewSize(width: bounds.width, height: bounds.height))
    }

    // Without these, a stack asking where this view's alignment guides are
    // has it place its view to find out, laying out all of it.
    func explicitAlignment(
        of guide: HorizontalAlignment, in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews,
        cache: inout ()
    ) -> CGFloat? { nil }

    func explicitAlignment(
        of guide: VerticalAlignment, in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews,
        cache: inout ()
    ) -> CGFloat? { nil }
}

/// A row of the chat: an item, or one part of a long reply.
private struct ChatLine: Identifiable {
    let item: ChatItem
    /// The part of the reply's text this row shows; nil for all the item.
    var part: Range<String.Index>?
    /// It's a part of a reply after its first.
    var continues = false
    let id: String
}

/// A finished part of the reply being written.
private struct StreamedPart: View, Equatable {
    let text: String

    var body: some View {
        MarkdownText(text)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The reply being written cut into parts as `ReplyParts` cuts a finished
/// one, so its parts are rows of the lazy list. A part, once followed by
/// another, stays as it is, so each call reads only what came after the
/// last of them.
@MainActor private final class StreamedParts {
    private var done: [String] = []
    /// Bytes of the reply the finished parts hold.
    private var length = 0

    /// The finished parts of `text`, and the part being written.
    func split(_ text: String) -> (done: [String], writing: String) {
        let utf8 = text.utf8
        if utf8.count < length || !done.isEmpty && !Self.continues(text, after: done[done.count - 1], at: length) {
            done.removeAll()
            length = 0
        }
        let start = utf8.index(utf8.startIndex, offsetBy: length)
        let parts = MarkdownText.parts(of: String(text[start...]), size: Self.size)
        for part in parts.dropLast() {
            done.append(String(part))
            length += part.utf8.count
        }
        return (done, parts.last.map(String.init) ?? "")
    }

    /// Whether `text` holds `last` just before `length`: it goes on from
    /// the same reply, not a new one as long.
    private static func continues(_ text: String, after last: String, at length: Int) -> Bool {
        let tail = last.utf8.suffix(64)
        let utf8 = text.utf8
        let end = utf8.index(utf8.startIndex, offsetBy: length)
        let start = utf8.index(end, offsetBy: -tail.count)
        return utf8[start..<end].elementsEqual(tail)
    }

    /// Bytes of Markdown in a part, as `ReplyParts` cuts them.
    private static let size = 500
}

/// A long reply cut into parts, each a row of the chat, so scrolling lays
/// out only the parts on screen however long the reply; each reply is cut
/// once.
@MainActor private final class ReplyParts {
    private var kept: [String: (length: Int, parts: [Range<String.Index>])] = [:]

    /// The chat's rows. A highlighted reply stays whole, so all of it is
    /// highlighted.
    func lines(of items: [ChatItem], highlighted: String?) -> [ChatLine] {
        var lines: [ChatLine] = []
        lines.reserveCapacity(items.count)
        for item in items {
            guard case .reply(let id, let text) = item, id != highlighted, text.utf8.count > Self.size else {
                lines.append(ChatLine(item: item, id: item.id))
                continue
            }
            let parts: [Range<String.Index>]
            if let kept = kept[id], kept.length == text.utf8.count {
                parts = kept.parts
            } else {
                parts = MarkdownText.parts(of: text, size: Self.size).map { $0.startIndex..<$0.endIndex }
                kept[id] = (text.utf8.count, parts)
            }
            for (index, part) in parts.enumerated() {
                lines.append(
                    ChatLine(item: item, part: part, continues: index > 0, id: index == 0 ? id : "\(id)/\(index)"))
            }
        }
        return lines
    }

    /// Bytes of Markdown in a part, about a screen of it.
    private static let size = 500
}

/// One row of the chat: a message, thoughts, a tool call or a note. Equal
/// when its item and what it shows are, whatever its actions, so a change
/// elsewhere in the session doesn't draw it again. It reads nothing from
/// the store itself; the store only takes its actions.
private struct ChatRow: View, Equatable {
    /// What the row shows of the session besides its item.
    struct Shows: Equatable {
        var highlighted = false
        /// The person's message is open to change.
        var editing = false
        /// The loaded thumbnails of the message's images, by name.
        var thumbnails: [String: Data] = [:]
        var thinkingOpen = false
        /// The tool call's output or change is open.
        var toolOpen = false
        /// The proposal the tool call made, shown as a card.
        var proposal: Proposal?
        var deciding = false
        var canStartWork = false
        /// Why the proposal the call made couldn't be read.
        var proposalFailure: String?
        var rewinding = false
        var archived = false
        /// The failure offers Try Again.
        var retryable = false
    }

    let item: ChatItem
    /// The part of a long reply the row shows; nil for all of the item.
    var part: Range<String.Index>?
    let shows: Shows
    let store: StoreOf<SessionFeature>
    let select: (String) -> Void

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.item == rhs.item && lhs.part == rhs.part && lhs.shows == rhs.shows
    }

    var body: some View {
        content
            .padding(shows.highlighted ? 6 : 0)
            .background {
                if shows.highlighted {
                    RoundedRectangle(cornerRadius: 8).fill(Color.yellow.opacity(0.18))
                }
            }
    }

    @ViewBuilder
    private var content: some View {
        switch item {
        case .person(let id, let text, let images):
            if shows.editing {
                MessageEditor(store: store)
            } else {
                MessageView(
                    role: .person, text: text, select: select,
                    attachments: {
                        if !images.isEmpty {
                            SentImages(names: images, thumbnails: shows.thumbnails) {
                                store.send(.thumbnailNeeded($0))
                            }
                        }
                    }
                )
                .contextMenu {
                    Button("Edit Message…") { store.send(.editTapped(id)) }
                    Button("Rewind to Before This…") { store.send(.rewindTapped(id)) }
                        .disabled(shows.rewinding || shows.archived)
                    Divider()
                    Button("Fork Before This With the Code") {
                        store.send(.forkTapped(before: id, chatOnly: false))
                    }
                    .disabled(store.forking)
                    Button("Fork Before This, Chat Only") {
                        store.send(.forkTapped(before: id, chatOnly: true))
                    }
                    .disabled(store.forking)
                }
            }
        case .reply(_, let text):
            if let part, part.upperBound <= text.endIndex {
                MarkdownText(String(text[part]))
                    .selectableOnMac()
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                MessageView(role: .assistant, text: text, select: select)
            }
        case .thinking(let id, let text):
            ThinkingDisclosure(
                thinking: ModelThinking(text: text), isExpanded: shows.thinkingOpen,
                toggle: { store.send(.thinkingToggled(id: id)) }, select: select, answering: true)
        case .tool(let call):
            if let proposal = shows.proposal {
                ProposalCard(proposal: proposal, deciding: shows.deciding, canStartWork: shows.canStartWork) {
                    store.send(.proposals($0))
                }
            } else {
                ToolCallView(
                    call: call, expanded: shows.toolOpen, toggle: { store.send(.toolToggled(id: call.id)) },
                    select: select)
                if let failure = shows.proposalFailure {
                    Label("Couldn’t read this proposal: \(failure)", systemImage: "exclamationmark.triangle")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        case .permission(_, let permission):
            PermissionCard(permission: permission, select: select)
        case .cost(_, let usage):
            Label(SessionView.describe(usage), systemImage: "gauge.with.dots.needle.33percent")
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
                        .disabled(shows.rewinding)
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
        case .failure(_, let message):
            HStack(alignment: .firstTextBaseline) {
                Label(message, systemImage: "exclamationmark.triangle")
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .textSelection(.enabled)
                if shows.retryable {
                    Spacer()
                    Button("Try Again") { store.send(.retryTapped) }
                        .disabled(store.retrying)
                        .help("Send your last message again")
                }
            }
        }
    }
}

/// The person's earlier message, open to change. Sending it forks, so
/// this session stays as it was. Its own view, so each key typed redraws
/// only this.
private struct MessageEditor: View {
    @Bindable var store: StoreOf<SessionFeature>

    var body: some View {
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
                    .disabled(store.forking || store.edited.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
    }
}

/// A tool the agent called: what it ran or wrote, the permission it asked
/// for, and its output, folded away until opened.
/// A tool call, opened to show its output or change. Whether it's open is
/// the session's, not the view's: the chat keeps each row's height by what
/// it shows, so a row that opened by itself kept the height it had closed.
private struct ToolCallView: View {
    let call: ToolCall
    let expanded: Bool
    let toggle: () -> Void
    /// Opens text to select from, for output too long to show in the chat.
    var select: (String) -> Void = { _ in }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button(action: toggle) {
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
                PermissionCard(permission: permission, select: select)
            }
            if expanded, let output = call.output, let preview = call.preview {
                outputView(output, preview)
                    .simultaneousGesture(collapse)
            }
        }
        .padding(10)
        .background(.fill.quaternary.opacity(0.5), in: .rect(cornerRadius: 12))
    }

    /// A double-click on what's open closes it, wherever it was scrolled
    /// to; dragging still selects.
    private var collapse: some Gesture {
        TapGesture(count: 2).onEnded { if expanded { toggle() } }
    }

    /// The output as the chat shows it, worked out when it came: its first
    /// 20,000 characters, scrolling in a box of its own when long, so
    /// opening and closing it doesn't lay the whole chat out again.
    @ViewBuilder
    private func outputView(_ output: String, _ preview: TextPreview) -> some View {
        let text = Text(preview.shown)
            .font(.caption.monospaced())
            .foregroundStyle(.secondary)
            .textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(8)
        VStack(alignment: .leading, spacing: 4) {
            if preview.isTall {
                ScrollView { text }
                    .frame(height: 320)
                    .background(.fill.quinary, in: .rect(cornerRadius: 8))
            } else {
                text.background(.fill.quinary, in: .rect(cornerRadius: 8))
            }
            if let length = preview.length {
                HStack {
                    Text("The first \(preview.shown.count.formatted()) of \(length.formatted()) characters")
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

/// The lines a write takes out, in red, and puts in, in green: those the
/// change kept, with a note of how many more there are.
struct ChangeView: View {
    let change: ToolCall.Change
    /// Past this many lines the change scrolls in a box of its own.
    private static let boxedLines = 24

    var body: some View {
        let count = change.removed.count + change.added.count
        let more = change.removedCount + change.addedCount - count
        VStack(alignment: .leading, spacing: 0) {
            if count > Self.boxedLines {
                // Lazy, so only the lines scrolled to are laid out.
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) { lines(count) }
                }
                .frame(height: 320)
            } else {
                lines(count)
            }
            if more > 0 {
                Text("\(more) more lines")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(6)
            }
        }
        .textSelection(.enabled)
        .clipShape(.rect(cornerRadius: 8))
        .accessibilityElement(children: .contain)
        .accessibilityLabel("\(change.removedCount) lines removed, \(change.addedCount) added")
    }

    /// The first `count` lines, the removed ones first, each read only as
    /// it's drawn.
    private func lines(_ count: Int) -> some View {
        ForEach(0..<count, id: \.self) { index in
            let removed = index < change.removed.count
            let line = removed ? change.removed[index] : change.added[index - change.removed.count]
            Text(verbatim: "\(removed ? "-" : "+") \(line)")
                .font(.caption.monospaced())
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 6)
                .background(removed ? Color.red.opacity(0.15) : Color.green.opacity(0.15))
                // Colour and glyph don't reach VoiceOver; the words do.
                .accessibilityLabel(removed ? "Removed: \(line)" : "Added: \(line)")
        }
    }
}

/// A request for permission: the command while the agent waits, answered
/// from the strip over the box, so it's asked in one place; then how it
/// was answered.
struct PermissionCard: View {
    let permission: Permission
    /// Opens text to read and select whole.
    var select: ((String) -> Void)?

    var body: some View {
        switch permission.answer {
        case nil:
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .top, spacing: 8) {
                    Image(systemName: "hand.raised")
                        .accessibilityHidden(true)
                    FullCommand(text: permission.summary, select: select)
                }
                Text("Waiting for you: Approve or Deny over the message box (⇧⌘A, ⇧⌘D).")
                    .font(.caption)
                    .foregroundStyle(.secondary)
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

/// What a request for permission would run or write: monospaced,
/// selectable, and scrolling past about twelve lines, so it can be read
/// before it's allowed. Past its first 20,000 characters the rest opens to
/// read and select.
struct FullCommand: View {
    let text: String
    var select: ((String) -> Void)?

    var body: some View {
        let preview = TextPreview(text)
        VStack(alignment: .leading, spacing: 4) {
            ScrollView {
                Text(preview.shown)
                    .font(.callout.monospaced())
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: 12 * 17)
            .fixedSize(horizontal: false, vertical: true)
            if preview.isCut, let select {
                Button("Show All") { select(text) }
                    .help("Only the first 20,000 characters show here; open all of them to read before you answer")
                    .buttonStyle(.link)
                    .font(.caption)
            }
        }
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
                let answering = store.answering.contains(id)
                if answering {
                    ProgressView().controlSize(.small)
                }
                Button("Deny") { store.send(.answerPermission(requestID: id, approved: false)) }
                    .keyboardShortcut("d", modifiers: [.command, .shift])
                    .disabled(answering)
                    .help("Deny (⇧⌘D)")
                Button("Approve") { store.send(.answerPermission(requestID: id, approved: true)) }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut("a", modifiers: [.command, .shift])
                    .disabled(answering)
                    .help("Approve (⇧⌘A)")
                Menu {
                    let widened: ApprovalMode = kind == .write ? .allowEdits : .allowEverything
                    Button(kind == .write ? "Allow Edits from Now On" : "Allow Everything from Now On") {
                        store.send(.approvalModePicked(widened))
                    }
                } label: {
                    Image(systemName: "chevron.down")
                        .accessibilityLabel("More Answers")
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
/// streams; that it's stopping, once told to; and before any turn, that a
/// message is on its way. Its own view, so each streamed word redraws only
/// this and not every row of the chat.
private struct TurnInProgress: View {
    let store: StoreOf<SessionFeature>
    @State private var parts = StreamedParts()

    var body: some View {
        if store.stopping {
            // What's written so far stays until the turn has stopped.
            if !store.streaming.isEmpty {
                reply(store.streaming)
            }
            status("Stopping…")
        } else if store.isWorking, let reason = store.waitReason {
            status(reason)
        } else if store.isWorking, !store.isWaiting {
            reply(store.streaming)
        } else if store.sending > 0, !store.isWorking {
            status("Sending…")
        }
    }

    /// The reply being written: its finished parts as rows of their own,
    /// drawn once, and only the part being written drawn again as it grows.
    @ViewBuilder
    private func reply(_ text: String) -> some View {
        let (done, writing) = parts.split(text)
        ForEach(done.indices, id: \.self) { index in
            StreamedPart(text: done[index]).equatable()
                .padding(.top, index > 0 ? -4 : 0)
        }
        ReplyInProgress(text: writing, thinking: nil, showsThinking: false, toggleThinking: {})
            .padding(.top, done.isEmpty ? 0 : -4)
    }

    private func status(_ text: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            ProgressView().controlSize(.small)
            Text(text).foregroundStyle(.secondary)
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
                    .disabled(store.stopping)
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
