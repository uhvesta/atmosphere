import ComposableArchitecture
import KhodrDaemonClient
import SwiftUI

/// Every project with its sessions as a tree of forks, each showing whether
/// it is working or waiting on the person, and where it runs.
package struct ProjectsView: View {
    @Bindable var store: StoreOf<ProjectsFeature>

    package init(store: StoreOf<ProjectsFeature>) {
        self.store = store
    }

    package var body: some View {
        list
            .overlay {
                if let failure = store.failure, store.projects.isEmpty, store.sessions.isEmpty {
                    ContentUnavailableView {
                        Label("Can't Reach Khodr's Daemon", systemImage: "bolt.horizontal.circle")
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
                    .help("Add a project for a repository on this Mac")
                }
            }
            .task { await store.send(.task).finish() }
            .sheet(item: $store.scope(state: \.destination?.newProject, action: \.destination.newProject)) {
                NewProjectView(store: $0)
            }
            .sheet(item: $store.scope(state: \.destination?.newSession, action: \.destination.newSession)) {
                NewSessionView(store: $0)
            }
            .sheet(item: $store.scope(state: \.destination?.rename, action: \.destination.rename)) {
                RenameSessionView(store: $0)
            }
            .sheet(item: $store.scope(state: \.destination?.compare, action: \.destination.compare)) {
                CompareView(store: $0)
            }
            .alert($store.scope(state: \.destination?.alert, action: \.destination.alert))
    }

    private var list: some View {
        List(selection: $store.selection.sending(\.sessionSelected)) {
            ForEach(store.projects) { project in
                Section {
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
                }
            }
            let unfiled = store.state.tree(in: nil)
            if !unfiled.isEmpty {
                Section("Other Sessions") {
                    tree(unfiled)
                }
            }
            if !store.state.archived.isEmpty {
                Section("Archived") {
                    ForEach(store.state.archived) { session in
                        row(session)
                    }
                }
            }
        }
        .listStyle(.sidebar)
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
        SessionRow(session: session)
            .tag(session.id)
            .contextMenu {
                Button("Rename…") { store.send(.renameTapped(session.id)) }
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

    var body: some View {
        HStack(spacing: 6) {
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
                Text(host)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .help("Runs on \(host)")
            }
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
            case .idle:
                EmptyView()
            }
        }
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
                        TextField("Repository", text: $row.path, prompt: Text("/path/to/repository"))
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
                    "Git repositories on this Mac. Each session works in its own copy of all of them, so the "
                        + "folders themselves are left as they are."
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

struct NewSessionView: View {
    @Bindable var store: StoreOf<NewSessionFeature>
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Form {
            Section {
                TextField("Name", text: $store.name, prompt: Text("What it's for"))
                Picker("Agent", selection: $store.backend) {
                    Text("Copilot").tag(Backend.copilot)
                    Text("Local Model").tag(Backend.local)
                    Text("Scripted").tag(Backend.scripted)
                }
                TextField("Model", text: $store.model, prompt: Text("The agent's default"))
                Picker("Approvals", selection: $store.approvalMode) {
                    Text("Ask Every Time").tag(ApprovalMode.askEveryTime)
                    Text("Allow Edits").tag(ApprovalMode.allowEdits)
                    Text("Allow Everything").tag(ApprovalMode.allowEverything)
                }
            } header: {
                Text("New Session in \(store.project.name)")
            } footer: {
                Text("It works in its own copy of \(Self.list(store.project.repositories.map(\.name))).")
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
                    .disabled(store.starting)
            }
        }
        .alert($store.scope(state: \.alert, action: \.alert))
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
