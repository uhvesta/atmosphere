import AppKit
import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// Every project with its sessions as a tree of forks, each showing whether
/// it is working or waiting on the person, and where it runs.
package struct ProjectsView: View {
    @Bindable var store: StoreOf<ProjectsFeature>

    package init(store: StoreOf<ProjectsFeature>) {
        self.store = store
    }

    // Reads only what's picked and what's searched for. What the list
    // holds is read by views of their own, so picking a session doesn't
    // work the whole tree out again, and a read of the list draws only
    // what it changed.
    package var body: some View {
        let _ = SidebarBodyRuns.ran(.sidebar)
        List(selection: selection) {
            SidebarItems(store: store)
        }
        .listStyle(.sidebar)
        .searchable(text: $store.search.sending(\.searchChanged), placement: .sidebar, prompt: "Search names")
        .overlay { SidebarNotice(store: store) }
        .toolbar {
            ToolbarItem {
                Button {
                    store.send(.addProjectTapped)
                } label: {
                    Label("Add Project", systemImage: "folder.badge.plus")
                }
                .help("Add a project for a repository on this Mac (⇧⌘N)")
            }
        }
        .modifier(Sheets(store: store))
    }

    /// What's picked. A pick is sent once the list's table is done telling
    /// of it: opening what was picked changes rows, and a table changed
    /// while it reports its selection warns of a reentrant operation and
    /// can draw rows wrong.
    private var selection: Binding<SidebarItem?> {
        Binding(
            get: { store.selection },
            set: { picked in
                Task { @MainActor in store.send(.selected(picked)) }
            })
    }
}

/// Over an empty sidebar: why it's empty, and what to do about it.
private struct SidebarNotice: View {
    let store: StoreOf<ProjectsFeature>

    var body: some View {
        if let failure = store.failure, store.projects.isEmpty, store.sessions.isEmpty {
            ContentUnavailableView {
                Label("Can't Reach Humata's Daemon", systemImage: "bolt.horizontal.circle")
            } description: {
                Text(failure)
            } actions: {
                Button("Try Again") { store.send(.refresh) }
            }
        } else if !store.loadedOnce {
            ProgressView("Reading your projects…")
                .controlSize(.small)
        } else if store.projects.isEmpty, store.sessions.isEmpty {
            ContentUnavailableView {
                Label("No Projects", systemImage: "folder")
            } description: {
                Text("Add a repository on this Mac to start working with an agent in it.")
            } actions: {
                Button("Add Project…") { store.send(.addProjectTapped) }
            }
        }
    }
}

/// The inbox's row. Its own view, so a change to what waits on the person
/// draws only its count again.
private struct InboxLabel: View {
    let store: StoreOf<ProjectsFeature>

    var body: some View {
        let _ = SidebarBodyRuns.ran(.inbox)
        // Everything in the inbox: sessions and admin actions.
        let waiting = store.inbox.count + store.approvals.count
        Label("Inbox", systemImage: "tray")
            .badge(store.blockedCount)
            .accessibilityLabel(
                Text("Inbox, \(store.blockedCount) blocked on you, \(waiting) in all"))
    }
}

/// The sidebar's rows: the inbox, hosts and search, then what the name
/// search found or every project with its sessions.
///
/// It holds the store and nothing else, so drawn again by the sidebar for
/// another pick it's the same as it was and isn't worked out again; with
/// the archived section's stored setting here it never was the same.
private struct SidebarItems: View {
    let store: StoreOf<ProjectsFeature>

    var body: some View {
        let _ = SidebarBodyRuns.ran(.list)
        InboxLabel(store: store)
            .tag(SidebarItem.inbox)
        Label("Hosts", systemImage: "server.rack")
            .tag(SidebarItem.hosts)
        Label("Search Sessions", systemImage: "text.magnifyingglass")
            .tag(SidebarItem.search)
        // With nothing listed the notice over the list says it; with a
        // list, the list is as it was last read.
        if let failure = store.failure, !(store.projects.isEmpty && store.sessions.isEmpty) {
            HStack {
                Label("Can't reach Humata's daemon", systemImage: "bolt.horizontal.circle")
                    .foregroundStyle(.orange)
                    .help(failure)
                Spacer()
                Button("Try Again") { store.send(.refresh) }
                    .buttonStyle(.borderless)
            }
            .selectionDisabled()
        }
        if let matches = store.state.matches {
            results(matches)
        } else {
            everything
        }
    }

    /// What the search found, or that it found nothing.
    @ViewBuilder
    private func results(_ matches: (projects: [Project], sessions: [Session])) -> some View {
        if matches.projects.isEmpty, matches.sessions.isEmpty {
            Text("No project or session is named like that.")
                .foregroundStyle(.secondary)
        }
        if !matches.projects.isEmpty {
            Section("Projects") {
                ForEach(matches.projects) { project in
                    Label(project.name, systemImage: "rectangle.split.3x1")
                        .tag(SidebarItem.board(project.id))
                }
            }
        }
        if !matches.sessions.isEmpty {
            Section("Sessions") {
                ForEach(matches.sessions) { session in
                    SidebarSessionRow.row(session, in: store)
                }
            }
        }
    }

    /// Every project, then the chats, then what's archived.
    @ViewBuilder
    private var everything: some View {
        Group {
            ForEach(store.state.sortedProjects) { project in
                Section {
                    Label("Board", systemImage: "rectangle.split.3x1")
                        .tag(SidebarItem.board(project.id))
                    tree(store.state.tree(in: project.id))
                } header: {
                    HStack {
                        Text(project.name)
                        Spacer()
                        Button {
                            store.send(.newSessionTapped(projectID: project.id))
                        } label: {
                            Label("New Session", systemImage: "plus")
                                .labelStyle(.iconOnly)
                        }
                        .buttonStyle(.borderless)
                        .help("Start a session in \(project.name)")
                    }
                    .contextMenu {
                        Button("New Session…") { store.send(.newSessionTapped(projectID: project.id)) }
                        Button("Plan With an Agent…") { store.send(.planTapped(projectID: project.id)) }
                        Button("Project Settings…") { store.send(.projectSettingsTapped(project.id)) }
                        Divider()
                        Button("Delete Project…", role: .destructive) {
                            store.send(.deleteProjectTapped(project.id))
                        }
                    }
                }
            }
            Section {
                tree(store.state.tree(in: nil))
            } header: {
                HStack {
                    Text("Chats")
                    Spacer()
                    Button {
                        store.send(.importTapped)
                    } label: {
                        Label("Import Session…", systemImage: "square.and.arrow.down")
                            .labelStyle(.iconOnly)
                    }
                    .buttonStyle(.borderless)
                    .help("Bring in a Claude Code or Copilot session started outside Humata, with its history")
                    Button {
                        store.send(.newChatTapped)
                    } label: {
                        Label("New Chat", systemImage: "plus")
                            .labelStyle(.iconOnly)
                    }
                    .buttonStyle(.borderless)
                    .help("Start a chat in no project, in a folder you pick or one of its own (⌘N)")
                }
            }
            ArchivedSessions(store: store)
        }
    }

    /// Every session in `nodes`, each indented under the one it was forked
    /// from, so the whole tree shows at once.
    private func tree(_ nodes: [SessionNode]) -> some View {
        ForEach(Self.flattened(nodes), id: \.session.id) { entry in
            SidebarSessionRow.row(entry.session, in: store)
                .padding(.leading, CGFloat(entry.depth) * 14)
        }
    }

    private static func flattened(_ nodes: [SessionNode], depth: Int = 0) -> [(session: Session, depth: Int)] {
        nodes.flatMap { node in
            [(node.session, depth)] + flattened(node.children ?? [], depth: depth + 1)
        }
    }
}

/// The archived sessions, folded away until opened, and as left.
private struct ArchivedSessions: View {
    let store: StoreOf<ProjectsFeature>
    @AppStorage("archivedShown") private var shown = false

    var body: some View {
        let archived = store.state.archived
        if !archived.isEmpty {
            Section("Archived (\(archived.count))", isExpanded: $shown) {
                ForEach(store.state.sorted(archived)) { session in
                    SidebarSessionRow.row(session, in: store)
                }
            }
        }
    }
}

/// A session's row in the sidebar with its menu. Equal when the session,
/// the time it's shown against and what holds it are, so the list drawn
/// again for another session leaves this row as it is. It reads nothing
/// from the store itself; the store only takes its actions.
private struct SidebarSessionRow: View, Equatable {
    let session: Session
    /// When the list was read, for how long ago it was active.
    let now: Date?
    /// What keeps it from being archived or moved now.
    let hold: SessionHold?
    let store: StoreOf<ProjectsFeature>

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.session == rhs.session && lhs.now == rhs.now && lhs.hold == rhs.hold
    }

    /// The session's row in the list, drawn again only when what it shows
    /// changes.
    static func row(_ session: Session, in store: StoreOf<ProjectsFeature>) -> some View {
        SidebarSessionRow(session: session, now: store.now, hold: store.state.hold(on: session), store: store)
            .equatable()
            .tag(SidebarItem.session(session.id))
    }

    var body: some View {
        let _ = SidebarBodyRuns.ran(.sessionRow)
        SessionRow(session: session, now: now, busy: hold == .busy)
            .contextMenu { menu }
    }

    /// Says why Archive and Move to Host are greyed out: a menu item's
    /// help doesn't always show.
    private var held: LocalizedStringKey? {
        switch hold {
        case .busy: "A change to it is still being made: wait for it to finish"
        case .inTurn: "Its turn is running: stop it to archive or move it"
        case nil: nil
        }
    }

    private var moveHelp: LocalizedStringKey {
        switch hold {
        case .busy: "Wait for the change being made to it to finish, then move it"
        case .inTurn: "Its turn is running; stop it or wait for it to end, then move it"
        case nil: "Run it on this Mac or one of your hosts"
        }
    }

    private var archiveHelp: LocalizedStringKey {
        switch hold {
        case .busy: "Wait for the change being made to it to finish, then archive it"
        case .inTurn: "Its turn is running; use Stop and Archive, or wait for it to end"
        case nil: "Put it away under Archived; its copy of the code stays until it's deleted"
        }
    }

    @ViewBuilder private var menu: some View {
        let busy = hold == .busy
        let inTurn = session.state == .working || session.state == .waitingForApproval
        Button("Rename…") { store.send(.renameTapped(session.id)) }
        if session.projectID != nil {
            Button("Move to Task…") { store.send(.moveTapped(session.id)) }
        }
        if !session.archived {
            Button("Move to Host…") { store.send(.moveToHostTapped(session.id)) }
                .disabled(hold != nil)
                .help(moveHelp)
        }
        Button(session.keeper ? "Unmark as Keeper" : "Mark as Keeper") {
            store.send(.keeperTapped(session.id, keeper: !session.keeper))
        }
        CompareMenu(session: session, store: store)
        Divider()
        if session.archived {
            Button("Unarchive") { store.send(.archiveTapped(session.id, archived: false)) }
                .disabled(busy)
            Button("Delete…", role: .destructive) { store.send(.deleteSessionTapped(session.id)) }
                .disabled(busy)
        } else {
            if let held { Text(held) }
            Button("Archive") { store.send(.archiveTapped(session.id, archived: true)) }
                .disabled(hold != nil)
                .help(archiveHelp)
            if inTurn {
                Button("Stop and Archive") { store.send(.stopAndArchiveTapped(session.id)) }
                    .disabled(busy)
                    .help(
                        busy
                            ? "Wait for the change being made to it to finish"
                            : "End its turn now, then archive it")
            }
        }
    }
}

/// The sessions a session can be compared with, in its menu. Its own
/// view: it reads the other sessions, which the row doesn't show, so a
/// change to them doesn't draw the row again.
private struct CompareMenu: View {
    let session: Session
    let store: StoreOf<ProjectsFeature>

    var body: some View {
        let others = store.state.comparable(to: session)
        if !others.isEmpty {
            Menu("Compare With") {
                ForEach(others) { other in
                    Button(other.title) { store.send(.compareTapped(session.id, with: other.id)) }
                }
            }
        }
    }
}

extension Session {
    /// Its name, or a stand-in when it has none.
    var title: String { name.isEmpty ? "Session" : name }
}

/// The one formatter for how long ago something was, shared by every row.
@MainActor private let relativeFormatter: RelativeDateTimeFormatter = {
    let formatter = RelativeDateTimeFormatter()
    formatter.unitsStyle = .abbreviated
    return formatter
}()

extension Date {
    /// How long before `now` it was, as "5 min. ago": the one way the
    /// sidebar and the inbox say it.
    @MainActor func relativeTime(to now: Date) -> String {
        relativeFormatter.localizedString(for: min(self, now), relativeTo: now)
    }
}

/// A session in the sidebar: its name, where it runs, whether it is the
/// keeper, and what it is doing.
struct SessionRow: View {
    let session: Session
    /// When the list was read, for how long ago it was active.
    var now: Date?
    /// Being moved, archived or deleted.
    var busy = false

    var body: some View {
        HStack(spacing: 6) {
            if session.turns > session.seenTurns {
                Circle()
                    .fill(.tint)
                    .frame(width: 7, height: 7)
                    .accessibilityLabel("Unread")
                    .help("It finished a turn you haven't read yet")
            }
            Label(session.title, systemImage: icon)
                .lineLimit(1)
                .help(help)
            if session.keeper {
                Image(systemName: "star.fill")
                    .foregroundStyle(.yellow)
                    .imageScale(.small)
                    .accessibilityLabel("Keeper")
                    .help("The session whose changes are the ones to keep")
            }
            Spacer()
            if let host = session.host {
                HStack(spacing: 2) {
                    if session.hostOffline {
                        Image(systemName: "wifi.slash")
                            .imageScale(.small)
                    }
                    Text(host)
                        .lineLimit(1)
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                .accessibilityElement(children: .combine)
                .accessibilityLabel(session.hostOffline ? "On \(host), offline" : "On \(host)")
                .help(
                    session.hostOffline
                        ? "Can't reach \(host); this is the session as it last was" : "Runs on \(host)")
            }
            if let since = since {
                Text(since)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                    .lineLimit(1)
                    .accessibilityLabel("Active \(since)")
            }
            if busy {
                ProgressView()
                    .controlSize(.mini)
                    .accessibilityLabel("Working on it")
            } else {
                SessionStateIcon(session: session)
            }
        }
    }

    /// How long ago it was last active, as "5 min. ago"; nil before anything
    /// happened in it.
    private var since: String? {
        guard let active = session.lastActive, let now else { return nil }
        return active.relativeTime(to: now)
    }

    /// What it works on, and for a side chat that it only reads.
    private var help: String {
        let works = session.itemTitle.map { String(localized: "Works on \($0)") }
        guard session.readOnly else { return works ?? "" }
        let reads = String(localized: "A side chat: it reads the code and the conversation, and changes nothing")
        return works.map { "\(reads). \($0)" } ?? reads
    }

    private var icon: String {
        // A side chat, a fork, a session another's agent started, or one
        // the person did.
        if session.readOnly { return "rectangle.righthalf.inset.filled" }
        if session.parentID != nil { return "arrow.triangle.branch" }
        return session.startedBy == nil ? "bubble.left.and.text.bubble.right" : "arrow.turn.down.right"
    }
}

struct NewProjectView: View {
    @Bindable var store: StoreOf<NewProjectFeature>
    @Environment(\.dismiss) private var dismiss
    @State private var picking = false

    var body: some View {
        Form {
            Section {
                TextField("Name", text: $store.name)
            }
            Section {
                ForEach($store.repositories) { $row in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            RepositoryField(
                                path: $row.path, row: row.id, search: store.scope(state: \.search, action: \.search))
                            if store.repositories.count > 1 {
                                Button {
                                    store.send(.removeRepositoryTapped(row.id))
                                } label: {
                                    Label("Remove", systemImage: "minus.circle")
                                        .labelStyle(.iconOnly)
                                }
                                .buttonStyle(.borderless)
                                .help("Leave this repository out of the project")
                            }
                        }
                        RepositorySuggestionList(row: row.id, search: store.scope(state: \.search, action: \.search))
                        if row.needsFullPath { FullPathNote() }
                    }
                }
                HStack {
                    Button("Choose…") { picking = true }
                    Button("Add Another") { store.send(.addRepositoryTapped) }
                }
            } header: {
                Text("Repositories")
            } footer: {
                Text(
                    "Git repositories on this Mac, or URLs to clone, such as a GitHub repository's; type to "
                        + "search those used before and those of the GitHub owners in Settings › Repositories. Each "
                        + "session works in its own copy of all of them, so the folders themselves are left as "
                        + "they are, and a URL's clone is fetched before each session starts."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            Section("Instructions") {
                TextEditor(text: $store.instructions)
                    .frame(minHeight: 80)
                    .accessibilityLabel("Instructions")
            }
        }
        .formStyle(.grouped)
        .frame(minWidth: 460, minHeight: 360)
        .fileImporter(
            isPresented: $picking, allowedContentTypes: [.folder], allowsMultipleSelection: true,
            onCompletion: picked
        )
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            if store.creating {
                ToolbarItem(placement: .status) {
                    ProgressView().controlSize(.small)
                }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button(store.creating ? "Adding…" : "Add Project") { store.send(.createTapped) }
                    .disabled(!store.canCreate)
            }
        }
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}

extension NewProjectView {
    private func picked(_ result: Result<[URL], any Error>) {
        guard case .success(let urls) = result else { return }
        for url in urls {
            store.send(.folderPicked(url))
        }
    }
}

/// Under a repository typed as neither a URL nor a path from /: why the
/// project can't be saved with it.
struct FullPathNote: View {
    var body: some View {
        Text(
            "Give its full path, starting with /, or a URL to clone. A path that starts with ~ or is relative to "
                + "another folder isn't followed."
        )
        .font(.caption)
        .foregroundStyle(.red)
    }
}

/// The models the agent offers, to fill the model in from.
struct ModelMenu: View {
    let models: [ModelChoice]
    /// The model typed or picked, named on the button when it's listed.
    var picked: String = ""
    let pick: (String) -> Void

    var body: some View {
        Menu {
            ForEach(models) { model in
                Button {
                    pick(model.id)
                } label: {
                    Text(model.name)
                    if let detail = model.detail {
                        Text(detail)
                    }
                }
            }
        } label: {
            Text(models.first { $0.id == picked.trimmingCharacters(in: .whitespaces) }?.name ?? "Models")
        }
        .fixedSize()
        .help("Pick one of the models the agent offers")
    }
}

/// The sheet to start a session in a project or on one of its items, or a
/// chat.
package struct NewSessionView: View {
    @Bindable var store: StoreOf<NewSessionFeature>
    @Environment(\.dismiss) private var dismiss
    @AppStorage(SettingsTab.key) private var settingsTab = SettingsTab.environments

    package init(store: StoreOf<NewSessionFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                TextField("Name", text: $store.name, prompt: Text("What it's for"))
                if store.project != nil, store.item == nil {
                    Toggle(isOn: $store.planning) {
                        Text("Plan Only")
                        Text("Proposes tasks and specs for you to apply, with no copy of the code, so it opens at once")
                    }
                }
                Picker("Agent", selection: $store.backend) {
                    Text("Claude Code").tag(Backend.claude)
                    Text("Copilot").tag(Backend.copilot)
                    Text("Local Model").tag(Backend.local)
                    // For flows and tests only; set by them, never picked.
                    if store.backend == .scripted {
                        Text("Scripted").tag(Backend.scripted)
                    }
                }
                LabeledContent("Model") {
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            TextField("Model", text: $store.model, prompt: Text("The agent's default"))
                                .labelsHidden()
                            if !store.models.isEmpty {
                                ModelMenu(models: store.models, picked: store.model) { store.model = $0 }
                            }
                        }
                        if let detail = store.pickedModel?.detail {
                            Text(detail)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        if let reason = store.modelsUnavailable {
                            Text(reason)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                                .help(reason)
                        }
                        if store.modelsLoading {
                            HStack(spacing: 6) {
                                ProgressView().controlSize(.small)
                                Text("Listing models…")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        } else if store.needsModel {
                            Text("Name one the model server runs; it has no default.")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                if let picked = store.effortModel {
                    EffortPicker(model: picked, fusion: store.fusion, effort: $store.effort)
                }
                if store.backend == .copilot {
                    Toggle(isOn: $store.experimental) {
                        Text("Experimental Features")
                        Text(
                            store.fusion
                                ? "Copilot's features still being tried out. Fusion needs them to pick local models."
                                : "Copilot's features still being tried out, which may change or break."
                        )
                    }
                }
                if store.takesModelServer {
                    TextField(
                        "Model Server", text: $store.modelServer,
                        prompt: Text(store.backend == .local ? "http://127.0.0.1:11434/v1" : "GitHub's models")
                    )
                    .help(
                        "An OpenAI-compatible server, such as Ollama's, so turns cost no tokens. Name its model above.")
                }
                if let failure = store.hostsFailure {
                    LabeledContent("Runs On") {
                        VStack(alignment: .leading, spacing: 4) {
                            Text("Couldn't read the hosts: \(failure)")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(3)
                                .help(failure)
                            if store.host != nil {
                                Text(
                                    "This session was going to run on the host you picked last. "
                                        + "Start waits until the hosts are read."
                                )
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            }
                            HStack {
                                Button("Try Again") { store.send(.hostsRetried) }
                                if store.host != nil {
                                    Button("Run on This Mac") { store.send(.thisMacPicked) }
                                }
                            }
                        }
                    }
                } else if !store.hosts.isEmpty {
                    Picker("Runs On", selection: $store.host) {
                        Text("This Mac").tag(RemoteHost.ID?.none)
                        ForEach(store.hosts) { host in
                            Text(host.name).tag(RemoteHost.ID?.some(host.id))
                        }
                    }
                } else if store.host != nil, !store.hostsRead {
                    LabeledContent("Runs On") {
                        HStack(spacing: 6) {
                            ProgressView().controlSize(.small)
                            Text("Reading the hosts…")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                if store.project == nil {
                    LabeledContent("Folder") {
                        HStack {
                            Text(store.folder.map { ($0 as NSString).abbreviatingWithTildeInPath } ?? "Its Own")
                                .lineLimit(1)
                                .truncationMode(.middle)
                                .foregroundStyle(store.folder == nil ? .secondary : .primary)
                            if store.folder != nil {
                                Button("Use Its Own") { store.folder = nil }
                            }
                            if store.host == nil {
                                Button("Choose…") { chooseFolder() }
                            }
                        }
                    }
                    if store.host != nil {
                        TextField(
                            "Folder on the Host",
                            text: Binding(get: { store.folder ?? "" }, set: { store.folder = $0.isEmpty ? nil : $0 }),
                            prompt: Text("Its own"))
                    }
                }
                Picker("Approvals", selection: $store.approvalMode) {
                    Text("Ask Every Time").tag(ApprovalMode.askEveryTime)
                    Text("Allow Edits").tag(ApprovalMode.allowEdits)
                    Text("Allow Everything").tag(ApprovalMode.allowEverything)
                    if ApprovalMode.offered(for: store.backend).contains(.auto) {
                        Text("Auto").tag(ApprovalMode.auto)
                    }
                }
                if let item = store.item, item.parentID != nil {
                    Picker("Specs", selection: $store.specs) {
                        Text("This Subtask's").tag(SpecScope.item)
                        Text("This Subtask's and Its Task's").tag(SpecScope.withParents)
                    }
                }
            } header: {
                if let item = store.item {
                    Text("New Session on \(item.title)")
                } else if let project = store.project {
                    Text("New Session in \(project.name)")
                } else {
                    Text("New Chat")
                }
            } footer: {
                Text(footer)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            if let clash = store.skillClash {
                Section {
                    Label {
                        Text(clash)
                    } icon: {
                        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.yellow)
                    }
                    SettingsLink {
                        Text("Pick in Settings › Skills…")
                    }
                    .simultaneousGesture(TapGesture().onEnded { settingsTab = SettingsTab.skills })
                } header: {
                    Text("Skills to Pick Between")
                }
            }
        }
        .formStyle(.grouped)
        .frame(minWidth: 420, minHeight: 280)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            if store.starting {
                ToolbarItem(placement: .status) {
                    HStack(spacing: 6) {
                        ProgressView().controlSize(.small)
                        Text(store.startingStatus)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button(store.starting ? "Starting…" : "Start") { store.send(.startTapped) }
                    .disabled(!store.canStart)
            }
        }
        .alert($store.scope(state: \.alert, action: \.alert))
        .task { await store.send(.task).finish() }
    }

    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.prompt = "Choose"
        panel.message = "The folder the chat works in. The agent can read and change its files."
        if panel.runModal() == .OK, let url = panel.url {
            store.folder = url.path
        }
    }

    private var footer: String {
        guard let project = store.project else {
            let place = store.host.map { " on \($0)" } ?? ""
            guard store.folder != nil else {
                return "It's a chat in no project. It gets an empty folder of its own\(place) for the files it "
                    + "writes, which you can read and comment on in Files."
            }
            return "It's a chat in no project. It works in the folder itself\(place), not a copy, so its changes "
                + "are made there as it makes them."
        }
        let names = Self.list(project.repositories.map(\.name))
        guard let host = store.host else { return "It works in its own copy of \(names)." }
        return "It works in its own copy of \(names) on \(host), cloned there from each repository's origin, "
            + "so commits not pushed yet aren't in it."
    }
}

extension NewSessionView {
    /// `names` as a sentence: "app", "app and api", "app, api and web".
    static func list(_ names: [String]) -> String {
        names.formatted(.list(type: .and))
    }
}

struct RenameSessionView: View {
    @Bindable var store: StoreOf<RenameSessionFeature>
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Form {
            TextField("Name", text: $store.name)
                .onSubmit { store.send(.saveTapped) }
        }
        .formStyle(.grouped)
        .frame(minWidth: 360)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Rename") { store.send(.saveTapped) }
                    .disabled(store.saving)
            }
        }
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}

struct MoveSessionView: View {
    @Bindable var store: StoreOf<MoveSessionFeature>
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Form {
            if let items = store.items {
                Picker("Works On", selection: $store.itemID) {
                    Text("The Project Itself").tag(String?.none)
                    ForEach(items) { item in
                        Text(item.fields.parentID == nil ? item.fields.title : "    \(item.fields.title)")
                            .tag(String?.some(item.id))
                    }
                }
            } else if let failure = store.itemsFailure {
                Section {
                    Label(failure, systemImage: "exclamationmark.triangle")
                    Button("Try Again") { store.send(.task) }
                } header: {
                    Text("Can't Read the Project's Tasks")
                }
            } else {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Reading the project's tasks…")
                        .foregroundStyle(.secondary)
                }
            }
        }
        .formStyle(.grouped)
        .frame(minWidth: 360)
        .task { await store.send(.task).finish() }
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Move") { store.send(.moveTapped) }
                    .disabled(store.saving || store.items == nil || store.itemID == store.session.itemID)
            }
        }
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}

/// The sidebar's sheets and alert, apart so the body type-checks in time.
private struct Sheets: ViewModifier {
    @Bindable var store: StoreOf<ProjectsFeature>

    func body(content: Content) -> some View {
        content
            .sheet(item: $store.scope(state: \.destination?.newProject, action: \.destination.newProject)) {
                NewProjectView(store: $0)
            }
            .sheet(item: $store.scope(state: \.destination?.newSession, action: \.destination.newSession)) {
                NewSessionView(store: $0)
            }
            .sheet(item: $store.scope(state: \.destination?.importSession, action: \.destination.importSession)) {
                ImportSessionView(store: $0)
            }
            .sheet(item: $store.scope(state: \.destination?.rename, action: \.destination.rename)) {
                RenameSessionView(store: $0)
            }
            .modifier(SessionSheets(store: store))
    }
}

/// The sheets about one session, and the alert.
private struct SessionSheets: ViewModifier {
    @Bindable var store: StoreOf<ProjectsFeature>

    func body(content: Content) -> some View {
        content
            .sheet(item: $store.scope(state: \.destination?.move, action: \.destination.move)) {
                MoveSessionView(store: $0)
            }
            .sheet(item: $store.scope(state: \.destination?.compare, action: \.destination.compare)) {
                CompareView(store: $0)
            }
            .sheet(
                item: $store.scope(state: \.destination?.projectSettings, action: \.destination.projectSettings)
            ) {
                ProjectSettingsView(store: $0)
            }
            .alert($store.scope(state: \.destination?.alert, action: \.destination.alert))
    }
}

/// Two sessions' changes side by side, each with a button to keep it.
struct CompareView: View {
    let store: StoreOf<CompareFeature>
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            if let failure = store.failure {
                ContentUnavailableView {
                    Label("Can't Compare", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(failure)
                } actions: {
                    Button("Try Again") { store.send(.task) }
                }
            } else {
                if let failure = store.keepFailure {
                    Label {
                        Text("Couldn't mark the session to keep, so neither changed: \(failure)")
                            .lineLimit(3)
                            .textSelection(.enabled)
                    } icon: {
                        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.yellow)
                    }
                    .font(.callout)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10)
                    Divider()
                }
                HStack(spacing: 0) {
                    side(store.left, store.leftChanges)
                    Divider()
                    side(store.right, store.rightChanges)
                }
            }
        }
        .frame(minWidth: 820, minHeight: 520)
        .task { await store.send(.task).finish() }
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { dismiss() }
            }
        }
    }

    private func side(_ session: Session, _ changes: CompareFeature.Changes?) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(session.title)
                    .font(.headline)
                    .lineLimit(1)
                Spacer()
                if session.keeper {
                    Label("Keeper", systemImage: "star.fill")
                        .foregroundStyle(.yellow)
                } else {
                    Button("Keep This") { store.send(.keepTapped(session.id)) }
                        .disabled(store.marking)
                        .help("Mark this session as the one whose changes to keep")
                }
            }
            .padding(10)
            Divider()
            if let changes {
                PatchView(changes: changes) { store.send(.showRestTapped(session.id)) }
            } else {
                VStack(spacing: 8) {
                    ProgressView()
                    Text("Reading its changes…")
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(maxWidth: .infinity)
    }
}

/// A session's changes, by repository, as diffs with added lines green and
/// removed ones red. Only the lines in view are drawn, and a long diff shows
/// its first lines until the rest are asked for.
struct PatchView: View {
    let changes: CompareFeature.Changes
    let showRest: () -> Void

    var body: some View {
        if changes.isEmpty {
            ContentUnavailableView("No Changes", systemImage: "doc.text", description: Text("Nothing changed yet."))
        } else {
            VStack(spacing: 0) {
                ScrollView([.vertical, .horizontal]) {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(changes.patches) { patch in
                            Section {
                                ForEach(0..<patch.shown, id: \.self) { index in
                                    let line = patch.lines[index]
                                    Text(line.isEmpty ? " " : line)
                                        .foregroundStyle(Self.color(line))
                                        .fixedSize()
                                }
                                .font(.caption.monospaced())
                            } header: {
                                // One past the cut has no lines to head yet.
                                if changes.patches.count > 1, patch.shown > 0 {
                                    Text(patch.name)
                                        .font(.subheadline.weight(.semibold))
                                        .padding(.vertical, 6)
                                }
                            }
                        }
                    }
                    .textSelection(.enabled)
                    .padding(10)
                }
                // A short diff starts at the top left, not in the middle.
                .defaultScrollAnchor(.topLeading)
                if changes.hidden > 0 {
                    Divider()
                    HStack {
                        Text(
                            "The first \(changes.shownCount.formatted()) of \(changes.lineCount.formatted()) lines "
                                + "are shown."
                        )
                        .foregroundStyle(.secondary)
                        Spacer()
                        Button("Show the Rest", action: showRest)
                            .help("Show every line of this session's changes")
                    }
                    .font(.callout)
                    .padding(8)
                }
            }
        }
    }

    private static func color(_ line: String) -> Color {
        if line.hasPrefix("+++") || line.hasPrefix("---") { return .secondary }
        if line.hasPrefix("+") { return .green }
        if line.hasPrefix("-") { return .red }
        if line.hasPrefix("@@") { return .purple }
        return .primary
    }
}

/// What a session is doing, as an icon: working, waiting on you or failed;
/// greyed as it last was while its host is offline, and nothing when idle.
package struct SessionStateIcon: View {
    var session: Session

    package init(session: Session) {
        self.session = session
    }

    package var body: some View {
        if session.hostOffline {
            lastState
        } else {
            state
        }
    }

    @ViewBuilder private var state: some View {
        switch session.state {
        case .working:
            ProgressView().controlSize(.small)
                .accessibilityLabel("Working")
        case .waitingForApproval:
            Image(systemName: "hand.raised.fill")
                .foregroundStyle(.orange)
                .accessibilityLabel("Waiting for you")
                .help("Waiting for you to answer a request")
        case .failed:
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.red)
                .accessibilityLabel("Failed")
        case .idle where session.asking:
            Image(systemName: "hand.raised.fill")
                .foregroundStyle(.orange)
                .accessibilityLabel("Waiting for you")
                .help("Asks you something, or to move its task")
        case .idle:
            EmptyView()
        }
    }

    /// The state it was in when its host last answered, greyed, since it
    /// may have changed since.
    @ViewBuilder private var lastState: some View {
        switch session.state {
        case .working:
            Image(systemName: "ellipsis.circle")
                .foregroundStyle(.secondary)
                .accessibilityLabel("Was working")
                .help("Was working when its host last answered")
        case .waitingForApproval:
            Image(systemName: "hand.raised")
                .foregroundStyle(.secondary)
                .accessibilityLabel("Was waiting for you")
                .help("Was waiting for you when its host last answered")
        case .failed:
            Image(systemName: "exclamationmark.triangle")
                .foregroundStyle(.secondary)
                .accessibilityLabel("Had failed")
        case .idle:
            EmptyView()
        }
    }
}
