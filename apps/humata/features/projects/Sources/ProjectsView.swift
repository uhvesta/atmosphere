import AppKit
import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// Every project with its sessions as a tree of forks, each showing whether
/// it is working or waiting on the person, and where it runs.
package struct ProjectsView: View {
    @Bindable var store: StoreOf<ProjectsFeature>
    /// Archived sessions stay folded away until opened, and as left.
    @AppStorage("archivedShown") private var archivedShown = false

    package init(store: StoreOf<ProjectsFeature>) {
        self.store = store
    }

    package var body: some View {
        list
            .overlay {
                if let failure = store.failure, store.projects.isEmpty, store.sessions.isEmpty {
                    ContentUnavailableView {
                        Label("Can't Reach Humata's Daemon", systemImage: "bolt.horizontal.circle")
                    } description: {
                        Text(failure)
                    } actions: {
                        Button("Try Again") { store.send(.refresh) }
                    }
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
            .toolbar {
                ToolbarItem {
                    Button {
                        store.send(.addProjectTapped)
                    } label: {
                        Label("Add Project", systemImage: "folder.badge.plus")
                    }
                    .keyboardShortcut("n", modifiers: [.command, .shift])
                    .help("Add a project for a repository on this Mac (⇧⌘N)")
                }
            }
            .modifier(Sheets(store: store))
    }

    private var list: some View {
        List(selection: $store.selection.sending(\.selected)) {
            Label("Inbox", systemImage: "tray")
                .badge(store.inbox.count)
                .tag(SidebarItem.inbox)
                .accessibilityLabel(
                    store.inbox.count == 1
                        ? Text("Inbox, 1 session waiting") : Text("Inbox, \(store.inbox.count) sessions waiting"))
            Label("Hosts", systemImage: "server.rack")
                .tag(SidebarItem.hosts)
            Label("Search Sessions", systemImage: "text.magnifyingglass")
                .tag(SidebarItem.search)
                .background {
                    // ⌘⇧F opens it from anywhere in the window.
                    Button("Search Sessions") { store.send(.selected(.search)) }
                        .keyboardShortcut("f", modifiers: [.command, .shift])
                        .opacity(0)
                        .accessibilityHidden(true)
                }
            if let matches = store.state.matches {
                results(matches)
            } else {
                everything
            }
        }
        .listStyle(.sidebar)
        .searchable(text: $store.search.sending(\.searchChanged), placement: .sidebar, prompt: "Search names")
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
                    row(session)
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
                    .keyboardShortcut("n", modifiers: .command)
                    .help("Start a chat in no project, in a folder you pick or one of its own (⌘N)")
                }
            }
            if !store.state.archived.isEmpty {
                Section("Archived (\(store.state.archived.count))", isExpanded: $archivedShown) {
                    ForEach(ProjectsFeature.State.sorted(store.state.archived)) { session in
                        row(session)
                    }
                }
            }
        }
    }

    /// Every session in `nodes`, each indented under the one it was forked
    /// from, so the whole tree shows at once.
    private func tree(_ nodes: [SessionNode]) -> some View {
        ForEach(Self.flattened(nodes), id: \.session.id) { entry in
            row(entry.session)
                .padding(.leading, CGFloat(entry.depth) * 14)
        }
    }

    private static func flattened(_ nodes: [SessionNode], depth: Int = 0) -> [(session: Session, depth: Int)] {
        nodes.flatMap { node in
            [(node.session, depth)] + flattened(node.children ?? [], depth: depth + 1)
        }
    }

    private func row(_ session: Session) -> some View {
        SessionRow(session: session, now: store.now)
            .tag(SidebarItem.session(session.id))
            .contextMenu {
                Button("Rename…") { store.send(.renameTapped(session.id)) }
                if session.projectID != nil {
                    Button("Move to Task…") { store.send(.moveTapped(session.id)) }
                }
                if !session.archived {
                    Button("Move to Host…") { store.send(.moveToHostTapped(session.id)) }
                        .disabled(session.state == .working || session.state == .waitingForApproval)
                }
                Button(session.keeper ? "Unmark as Keeper" : "Mark as Keeper") {
                    store.send(.keeperTapped(session.id, keeper: !session.keeper))
                }
                let others = store.state.comparable(to: session)
                if !others.isEmpty {
                    Menu("Compare With") {
                        ForEach(others) { other in
                            Button(other.title) { store.send(.compareTapped(session.id, with: other.id)) }
                        }
                    }
                }
                Divider()
                if session.archived {
                    Button("Unarchive") { store.send(.archiveTapped(session.id, archived: false)) }
                    Button("Delete…", role: .destructive) { store.send(.deleteSessionTapped(session.id)) }
                } else {
                    Button("Archive") { store.send(.archiveTapped(session.id, archived: true)) }
                        .disabled(session.state == .working || session.state == .waitingForApproval)
                }
            }
    }
}

extension Session {
    /// Its name, or a stand-in when it has none.
    var title: String { name.isEmpty ? "Session" : name }
}

/// A session in the sidebar: its name, where it runs, whether it is the
/// keeper, and what it is doing.
struct SessionRow: View {
    let session: Session
    /// When the list was read, for how long ago it was active.
    var now: Date?

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
            if session.hostOffline {
                lastState
            } else {
                state
            }
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

    /// How long ago it was last active, as "5 min ago"; nil before anything
    /// happened in it.
    private var since: String? {
        guard let active = session.lastActive, let now else { return nil }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .abbreviated
        return formatter.localizedString(for: min(active, now), relativeTo: now)
    }

    private var icon: String {
        session.parentID == nil ? "bubble.left.and.text.bubble.right" : "arrow.triangle.branch"
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
                    HStack {
                        TextField(
                            "Repository", text: $row.path,
                            prompt: Text("/path/to/repository or https://github.com/owner/name")
                        )
                        .labelsHidden()
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
                }
                HStack {
                    Button("Choose…") { picking = true }
                    Button("Add Another") { store.send(.addRepositoryTapped) }
                }
            } header: {
                Text("Repositories")
            } footer: {
                Text(
                    "Git repositories on this Mac, or URLs to clone, such as a GitHub repository's. Each "
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
            ToolbarItem(placement: .confirmationAction) {
                Button("Add Project") { store.send(.createTapped) }
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

/// The models the agent offers, to fill the model in from.
private struct ModelMenu: View {
    let models: [ModelChoice]
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
            Text("Models")
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
                                ModelMenu(models: store.models) { store.model = $0 }
                            }
                        }
                        if let reason = store.modelsUnavailable {
                            Text(reason)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                                .help(reason)
                        }
                        if store.needsModel {
                            Text("Name one the model server runs; it has no default.")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
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
                if !store.hosts.isEmpty {
                    Picker("Runs On", selection: $store.host) {
                        Text("This Mac").tag(RemoteHost.ID?.none)
                        ForEach(store.hosts) { host in
                            Text(host.name).tag(RemoteHost.ID?.some(host.id))
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
        }
        .formStyle(.grouped)
        .frame(minWidth: 420, minHeight: 280)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Start") { store.send(.startTapped) }
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
            } else {
                ProgressView()
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

    private func side(_ session: Session, _ changes: [RepositoryChanges]?) -> some View {
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
                PatchView(changes: changes)
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(maxWidth: .infinity)
    }
}

/// A session's changes, by repository, as diffs with added lines green and
/// removed ones red.
struct PatchView: View {
    let changes: [RepositoryChanges]

    var body: some View {
        if changes.allSatisfy({ $0.patch.isEmpty }) {
            ContentUnavailableView("No Changes", systemImage: "doc.text", description: Text("Nothing changed yet."))
        } else {
            ScrollView([.vertical, .horizontal]) {
                VStack(alignment: .leading, spacing: 12) {
                    ForEach(changes, id: \.name) { repository in
                        if changes.count > 1 {
                            Text(repository.name)
                                .font(.subheadline.weight(.semibold))
                        }
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(
                                Array(
                                    repository.patch.split(separator: "\n", omittingEmptySubsequences: false)
                                        .enumerated()), id: \.offset
                            ) { _, line in
                                Text(line.isEmpty ? " " : String(line))
                                    .foregroundStyle(Self.color(line))
                            }
                        }
                        .font(.caption.monospaced())
                        .textSelection(.enabled)
                    }
                }
                .padding(10)
            }
        }
    }

    private static func color(_ line: Substring) -> Color {
        if line.hasPrefix("+++") || line.hasPrefix("---") { return .secondary }
        if line.hasPrefix("+") { return .green }
        if line.hasPrefix("-") { return .red }
        if line.hasPrefix("@@") { return .purple }
        return .primary
    }
}
