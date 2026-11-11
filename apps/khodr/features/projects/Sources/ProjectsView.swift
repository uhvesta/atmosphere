import ComposableArchitecture
import KhodrDaemonClient
import SwiftUI

/// Every project with its sessions, each showing whether it is working or
/// waiting on the person.
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
    }

    private var list: some View {
        List(selection: $store.selection.sending(\.sessionSelected)) {
            ForEach(store.projects) { project in
                Section {
                    ForEach(store.state.sessions(in: project.id)) { session in
                        SessionRow(session: session).tag(session.id)
                    }
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
            if !store.state.unfiled.isEmpty {
                Section("Other Sessions") {
                    ForEach(store.state.unfiled) { session in
                        SessionRow(session: session).tag(session.id)
                    }
                }
            }
        }
        .listStyle(.sidebar)
    }
}

/// A session in the sidebar: its name and what it is doing.
struct SessionRow: View {
    let session: Session

    var body: some View {
        HStack(spacing: 6) {
            Label(session.name.isEmpty ? "Session" : session.name, systemImage: icon)
                .lineLimit(1)
            Spacer()
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
                LabeledContent("Repository") {
                    HStack {
                        TextField("Repository", text: $store.repository, prompt: Text("/path/to/repository"))
                            .labelsHidden()
                        Button("Choose…") { picking = true }
                    }
                }
                TextField("Name", text: $store.name)
            } footer: {
                Text("A folder on this Mac. Its sessions work in it.")
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
        .frame(minWidth: 460, minHeight: 320)
        .fileImporter(isPresented: $picking, allowedContentTypes: [.folder]) { result in
            if case .success(let url) = result { store.send(.folderPicked(url)) }
        }
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
                Text("It works in \(store.project.repository).")
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
