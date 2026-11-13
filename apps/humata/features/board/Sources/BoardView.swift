import Board
import ChatUI
import ComposableArchitecture
import HumataDaemonClient
import HumataProjects
import HumataReview
import HumataSession
import SwiftUI

extension Priority {
    var title: String {
        switch self {
        case .none: String(localized: "None")
        case .low: String(localized: "Low")
        case .medium: String(localized: "Medium")
        case .high: String(localized: "High")
        case .urgent: String(localized: "Urgent")
        }
    }
}

/// A project's board: a column per status. Drag a card to another column,
/// or use its context menu, to set its status; tap it to open its page.
package struct BoardView: View {
    @Bindable var store: StoreOf<BoardFeature>

    package init(store: StoreOf<BoardFeature>) {
        self.store = store
    }

    package var body: some View {
        ScrollView(.horizontal) {
            HStack(alignment: .top, spacing: 12) {
                ForEach(ItemStatus.allCases, id: \.self) { status in
                    column(status)
                }
            }
            .padding()
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            VStack(spacing: 0) {
                if let failure = store.actionFailure {
                    HStack {
                        Label(failure, systemImage: "exclamationmark.triangle.fill")
                            .textSelection(.enabled)
                        Spacer()
                        Button("Close", systemImage: "xmark") { store.send(.actionFailureDismissed) }
                            .labelStyle(.iconOnly)
                            .buttonStyle(.borderless)
                            .help("Close this message")
                    }
                    .font(.callout)
                    .padding(8)
                    .background(.bar)
                }
                if let failure = store.failure {
                    Label(failure, systemImage: "exclamationmark.triangle.fill")
                        .font(.callout)
                        .padding(8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(.bar)
                }
                if store.failure == nil, let failure = store.proposals.listFailure {
                    Label(
                        "Couldn’t read the agents’ proposals: \(failure)",
                        systemImage: "exclamationmark.triangle.fill"
                    )
                    .font(.callout)
                    .padding(8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.bar)
                }
                let proposed = store.proposals.pending
                if !proposed.isEmpty {
                    outsideProposals(proposed)
                }
            }
        }
        .overlay(alignment: .bottom) {
            if let undo = store.undo {
                UndoToastView(message: undo.message) { store.send(.undoTapped) }
                    .padding()
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.default, value: store.undo)
        .navigationTitle(store.project.name)
        .toolbar {
            ToolbarItem {
                Button("Undo", systemImage: "arrow.uturn.backward") { store.send(.undoTapped) }
                    .keyboardShortcut("z", modifiers: .command)
                    .help("Undo the last change you made to this board here; the History has every change")
            }
            ToolbarItem {
                Button("History", systemImage: "clock.arrow.circlepath") { store.send(.logTapped) }
                    .help("See and revert the board's changes")
            }
            ToolbarItem {
                Button("Import Issues", systemImage: "square.and.arrow.down") { store.send(.importIssuesTapped) }
                    .disabled(store.importing)
                    .help("Make a task of each open GitHub issue of the project's repositories not imported yet")
            }
            ToolbarItem {
                Button("Add Task", systemImage: "plus") { store.send(.addTapped(parent: nil)) }
            }
        }
        .alert($store.scope(state: \.destination?.alert, action: \.destination.alert))
        .alert($store.scope(state: \.proposals.alert, action: \.proposals.alert))
        .task { await store.send(.task).finish() }
        .sheet(isPresented: editingProposal) {
            ProposalEditor(store: store.scope(state: \.proposals, action: \.proposals))
        }
        .sheet(item: $store.scope(state: \.destination?.item, action: \.destination.item)) {
            ItemView(store: $0)
        }
        .sheet(item: $store.scope(state: \.destination?.newItem, action: \.destination.newItem)) {
            NewItemView(store: $0)
        }
        .sheet(item: $store.scope(state: \.destination?.log, action: \.destination.log)) {
            BoardLogView(store: $0)
        }
    }

    /// What agents outside Humata proposed through `humata mcp`, waiting
    /// for the person to apply, edit or decline it.
    private func outsideProposals(_ proposed: [Proposal]) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(
                proposed.count == 1
                    ? "1 proposal from an outside agent" : "\(proposed.count) proposals from outside agents"
            )
            .font(.caption.weight(.medium))
            .foregroundStyle(.secondary)
            ScrollView(.horizontal) {
                HStack(alignment: .top, spacing: 8) {
                    ForEach(proposed) { proposal in
                        ProposalCard(
                            proposal: proposal, deciding: store.proposals.deciding.contains(proposal.id),
                            canStartWork: false
                        ) { store.send(.proposals($0)) }
                        .frame(width: 320)
                    }
                }
            }
        }
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.bar)
    }

    private var editingProposal: Binding<Bool> {
        Binding(
            get: { store.proposals.editing != nil },
            set: { if !$0 { store.send(.proposals(.editCancelled)) } })
    }

    private func column(_ status: ItemStatus) -> some View {
        let cards = store.state.cards(in: status)
        return BoardColumn(
            title: status.title, count: cards.count,
            countLabel: cards.count == 1
                ? Text("\(status.title), 1 item") : Text("\(status.title), \(cards.count) items")
        ) {
            ForEach(cards) { item in
                card(item)
            }
        } footer: {
            if status == .todo {
                Button("Add Task", systemImage: "plus") { store.send(.addTapped(parent: nil)) }
                    .buttonStyle(.borderless)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 8)
            }
        } onDrop: { ids in
            let known = ids.filter { store.items[id: $0] != nil }
            for id in known {
                store.send(.dropped(id, on: status))
            }
            return !known.isEmpty
        }
    }

    private func card(_ item: Item) -> some View {
        BoardCard(id: item.id) {
            ItemCard(
                item: item, parent: store.state.parent(of: item), sessions: store.state.sessions(on: item),
                starting: store.starting.contains(item.id), startingStatus: store.project.startingStatus)
        } menu: {
            ForEach(ItemStatus.allCases.filter { $0 != item.status }, id: \.self) { status in
                Button("Move to \(status.title)") { store.send(.dropped(item.id, on: status)) }
            }
            Divider()
            Button("Start Work") { store.send(.startWorkTapped(item.id)) }
                .disabled(store.starting.contains(item.id))
                .help("Start a session on it with your last New Session settings, told to do what its spec says")
            if item.parentID == nil {
                Button("Add Subtask…") { store.send(.addTapped(parent: item.id)) }
            }
            Divider()
            Button("Delete…", role: .destructive) { store.send(.deleteTapped(item.id)) }
        } onTap: {
            store.send(.cardTapped(item.id))
        }
    }
}

/// A card's contents: the task a subtask is under, its title, its
/// priority, and what its sessions are doing.
private struct ItemCard: View {
    var item: Item
    var parent: Item?
    var sessions: [Session]
    var starting = false
    var startingStatus = "Starting…"

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let parent {
                Text(parent.title)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .accessibilityLabel("Subtask of \(parent.title)")
            }
            HStack(alignment: .firstTextBaseline) {
                Text(item.title).font(.callout)
                Spacer()
                if item.priority != .none {
                    PriorityBadge(priority: item.priority)
                }
            }
            let open = sessions.filter { !$0.archived }
            if starting {
                HStack(spacing: 4) {
                    ProgressView().controlSize(.mini)
                    Text(startingStatus).lineLimit(1).truncationMode(.middle)
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            } else if !open.isEmpty {
                HStack(spacing: 4) {
                    Image(systemName: "bubble.left.and.text.bubble.right")
                    Text(open.count == 1 ? "1 session" : "\(open.count) sessions")
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            // Sessions move a card set to To Do as they go; only a status
            // the person picked gets a note saying it is set aside.
            if item.status != item.fields.status, item.fields.status != .todo {
                Text("Set to \(item.fields.status.title); its sessions show it here")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
        }
    }
}

struct PriorityBadge: View {
    var priority: Priority

    var body: some View {
        Text(priority.title)
            .font(.caption2.bold())
            .padding(.horizontal, 5)
            .padding(.vertical, 1)
            .background(Capsule().fill(color.opacity(0.2)))
            .foregroundStyle(color)
            .accessibilityLabel("\(priority.title) priority")
    }

    private var color: Color {
        switch priority {
        case .urgent: .red
        case .high: .orange
        case .medium: .yellow
        case .low, .none: .secondary
        }
    }
}

/// A spec as Markdown to write, or rendered the way sessions' people read it.
private struct SpecEditor: View {
    @Binding var spec: String
    @Binding var previewing: Bool
    let minHeight: CGFloat

    var body: some View {
        Picker("Spec", selection: $previewing) {
            Text("Write").tag(false)
            Text("Preview").tag(true)
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        if previewing {
            Group {
                if spec.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Text("Nothing to preview yet.").foregroundStyle(.secondary)
                } else {
                    MarkdownText(spec).textSelection(.enabled)
                }
            }
            .frame(maxWidth: .infinity, minHeight: minHeight, alignment: .topLeading)
        } else {
            TextEditor(text: $spec)
                .font(.body.monospaced())
                .frame(minHeight: minHeight)
        }
    }
}

/// An item's page: its status, priority and spec, its subtasks, and its
/// sessions with what each changed.
struct ItemView: View {
    @Bindable var store: StoreOf<ItemFeature>
    @Environment(\.dismiss) private var dismiss

    /// The title and spec are being edited, so what would leave the page
    /// is off until they're saved or cancelled.
    private var editing: Bool { store.draft != nil }

    var body: some View {
        Form {
            Section {
                if let parent = store.parent {
                    LabeledContent("Task") {
                        Button(parent.title) { store.send(.itemTapped(parent.id)) }
                            .buttonStyle(.link)
                            .disabled(editing)
                            .help("Open \(parent.title)")
                    }
                }
                Picker(
                    "Status",
                    selection: Binding(get: { store.item.fields.status }, set: { store.send(.statusPicked($0)) })
                ) {
                    ForEach(ItemStatus.allCases, id: \.self) { Text($0.title).tag($0) }
                }
                if store.item.status != store.item.fields.status {
                    LabeledContent("On the Board", value: store.item.status.title)
                }
                Picker(
                    "Priority",
                    selection: Binding(get: { store.item.priority }, set: { store.send(.priorityPicked($0)) })
                ) {
                    ForEach(Priority.allCases, id: \.self) { Text($0.title).tag($0) }
                }
            } header: {
                // A sheet on the Mac has no title bar to show the title in.
                Text(store.item.title)
                    .font(.title2.bold())
                    .foregroundStyle(.primary)
            }
            Section("Spec") {
                if let draft = store.draft {
                    TextField(
                        "Title", text: Binding(get: { draft.title }, set: { store.draft?.title = $0 }))
                    SpecEditor(
                        spec: Binding(get: { draft.spec }, set: { store.draft?.spec = $0 }),
                        previewing: Binding(get: { draft.previewing }, set: { store.draft?.previewing = $0 }),
                        minHeight: 160)
                } else if store.item.spec.isEmpty {
                    Text("No spec yet. Edit to write what the work is and when it's done.")
                        .foregroundStyle(.secondary)
                } else {
                    SpecCommentsView(
                        store: store.scope(state: \.comments, action: \.comments), spec: store.item.spec,
                        sessions: store.commentTargets)
                }
            }
            if store.item.parentID == nil {
                Section("Subtasks") {
                    ForEach(store.subtasks) { subtask in
                        Button {
                            store.send(.itemTapped(subtask.id))
                        } label: {
                            LabeledContent(subtask.title, value: subtask.status.title)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .help("Open \(subtask.title)")
                    }
                    Button("Add Subtask…", systemImage: "plus") { store.send(.addSubtaskTapped) }
                        .buttonStyle(.borderless)
                }
                .disabled(editing)
            }
            if !store.pullRequests.isEmpty {
                Section {
                    ForEach(store.pullRequests) { pull in
                        PullRequestRow(pull: pull)
                    }
                } header: {
                    HStack {
                        Text("Pull Requests")
                        Spacer()
                        if store.pullsReadOnRefreshOnly {
                            Text("Read only when you refresh")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Button("Refresh", systemImage: "arrow.clockwise") { store.send(.refreshPullRequestsTapped) }
                            .labelStyle(.iconOnly)
                            .buttonStyle(.borderless)
                            .disabled(store.readingPullRequests)
                            .help(
                                store.pullsReadOnRefreshOnly
                                    ? "Read them again from GitHub; Settings › GitHub has Humata read them only then"
                                    : "Read them again from GitHub; Humata also does every few minutes")
                    }
                }
            }
            Section("Sessions") {
                ForEach(store.sortedSessions) { session in
                    Button {
                        store.send(.sessionTapped(session.id))
                    } label: {
                        HStack {
                            Text(session.name.isEmpty ? "Session" : session.name)
                                .foregroundStyle(session.archived ? .secondary : .primary)
                            if session.archived {
                                Text("Archived")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            SessionStateIcon(session: session)
                            if let files = store.changedFiles[session.id] {
                                Text(files == 1 ? "1 file changed" : "\(files) files changed")
                                    .foregroundStyle(.secondary)
                            }
                            Image(systemName: "chevron.right").foregroundStyle(.tertiary)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
                HStack {
                    Button(store.startingWork ? store.project.startingStatus : "Start Work", systemImage: "play.fill") {
                        store.send(.startWorkTapped)
                    }
                    .help("Start a session with your last New Session settings, told to do what the spec says")
                    .buttonStyle(.borderless)
                    .disabled(store.startingWork)
                    if store.startingWork {
                        ProgressView().controlSize(.small)
                    }
                }
                Button("Start Session…", systemImage: "play") { store.send(.startSessionTapped) }
                    .buttonStyle(.borderless)
            }
            .disabled(editing)
        }
        .formStyle(.grouped)
        .frame(minWidth: 520, minHeight: 560)
        .navigationTitle(store.item.title)
        .toolbar {
            if store.draft == nil {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button("Edit") { store.send(.editTapped) }
                }
                ToolbarItem(placement: .destructiveAction) {
                    Button("Delete…", role: .destructive) { store.send(.deleteTapped) }
                }
            } else {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { store.send(.cancelEditTapped) }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { store.send(.saveTapped) }
                        .disabled(store.saving)
                }
            }
        }
        .task { await store.send(.task).finish() }
        .sheet(item: $store.scope(state: \.destination?.newSession, action: \.destination.newSession)) {
            NewSessionView(store: $0)
        }
        .sheet(item: $store.scope(state: \.destination?.newSubtask, action: \.destination.newSubtask)) {
            NewItemView(store: $0)
        }
        .alert($store.scope(state: \.destination?.alert, action: \.destination.alert))
        .alert($store.scope(state: \.comments.alert, action: \.comments.alert))
    }
}

struct NewItemView: View {
    @Bindable var store: StoreOf<NewItemFeature>
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Form {
            Section {
                TextField("Title", text: $store.title)
                Picker("Priority", selection: $store.priority) {
                    ForEach(Priority.allCases, id: \.self) { Text($0.title).tag($0) }
                }
            } header: {
                if let parent = store.parent {
                    Text("New Subtask of \(parent.title)")
                } else {
                    Text("New Task in \(store.project.name)")
                }
            }
            Section {
                SpecEditor(spec: $store.spec, previewing: $store.previewingSpec, minHeight: 140)
            } header: {
                Text("Spec")
            } footer: {
                Text("Markdown. Sessions on it read it ahead of their instructions.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .frame(minWidth: 460, minHeight: 360)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Add") { store.send(.createTapped) }
                    .disabled(!store.canCreate)
            }
        }
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}

/// What was just changed on the board, with Undo.
private struct UndoToastView: View {
    let message: String
    let undo: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Text(message)
                .lineLimit(2)
            Button("Undo", action: undo)
                .buttonStyle(.borderless)
                .fontWeight(.semibold)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(.regularMaterial, in: .capsule)
        .shadow(radius: 4, y: 2)
    }
}

/// The board's journal: each change, who made it, and what it changed.
struct BoardLogView: View {
    @Bindable var store: StoreOf<BoardLogFeature>
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            // A sheet on the Mac shows no title or navigation toolbar items,
            // so the title and the filter head the list.
            HStack {
                Text("Board History").font(.headline)
                Spacer()
                Picker("Made by", selection: $store.author) {
                    Text("Everyone").tag(BoardAuthor?.none)
                    ForEach(store.authors, id: \.self) { author in
                        Text(store.state.name(of: author)).tag(BoardAuthor?.some(author))
                    }
                }
                .fixedSize()
                .help("Show only the changes one author made")
            }
            .padding()
            Divider()
            List {
                if store.changes.isEmpty {
                    Text("Nothing has changed on this board yet.")
                        .foregroundStyle(.secondary)
                }
                ForEach(store.shown) { change in
                    row(change)
                }
            }
        }
        .frame(minWidth: 520, minHeight: 400)
        .navigationTitle("Board History")
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { dismiss() }
            }
        }
        .task { await store.send(.task).finish() }
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    private func row(_ change: BoardChange) -> some View {
        HStack(alignment: .firstTextBaseline) {
            VStack(alignment: .leading, spacing: 3) {
                Text(BoardLogFeature.describe(change, items: store.items))
                HStack(spacing: 4) {
                    Text(store.state.name(of: change.author))
                    if change.grantID != nil {
                        Text("· under a grant")
                            .help("An admin action another program asked for, run under leave you gave")
                    } else if change.approvalID != nil {
                        Text("· approved by you")
                            .help("An admin action another program asked for, which you approved")
                    }
                    Text("·")
                    Text(change.time.formatted(date: .abbreviated, time: .shortened))
                    if let revert = store.state.revert(of: change) {
                        Text("·")
                        Text("Reverted by \(store.state.name(of: revert.author))")
                            .foregroundStyle(.orange)
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            Spacer()
            Button("Revert") { store.send(.revertTapped(change.id)) }
                .disabled(store.reverting != nil)
                .help("Put back what this change changed; fields changed again since are kept")
        }
    }
}

extension BoardLogFeature {
    /// What a change did, in a sentence: "Added login", "Moved login from
    /// To Do to Done", "Edited login's spec".
    package static func describe(_ change: BoardChange, items: IdentifiedArrayOf<Item>) -> String {
        let title = change.after?.title ?? change.before?.title ?? items[id: change.itemID]?.title ?? change.itemID
        let what: String
        switch (change.before, change.after) {
        case (nil, _): what = String(localized: "Added \(title)")
        case (_, nil): what = String(localized: "Removed \(title)")
        case (let before?, let after?):
            var parts: [String] = []
            if before.title != after.title {
                parts.append(String(localized: "Renamed \(before.title) to \(after.title)"))
            }
            if before.status != after.status {
                parts.append(
                    String(localized: "Moved \(title) from \(before.status.title) to \(after.status.title)"))
            }
            if before.priority != after.priority {
                parts.append(String(localized: "Set \(title)'s priority to \(after.priority.title)"))
            }
            if before.spec != after.spec {
                parts.append(String(localized: "Edited \(title)'s spec"))
            }
            what = parts.isEmpty ? String(localized: "Changed \(title)") : parts.joined(separator: "; ")
        }
        return change.reverts == nil ? what : String(localized: "Reverted: \(what)")
    }
}
