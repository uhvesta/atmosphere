import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// The hosts beside the selected one's health, variables and setup script.
package struct HostsView: View {
    @Bindable var store: StoreOf<HostsFeature>

    package init(store: StoreOf<HostsFeature>) {
        self.store = store
    }

    package var body: some View {
        HStack(spacing: 0) {
            List(selection: $store.selection.sending(\.selected)) {
                ForEach(store.hosts) { host in
                    HostRow(host: host, health: store.health[host.id])
                        .tag(host.id)
                }
            }
            .frame(width: 220)
            Divider()
            detail
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .overlay {
            if let failure = store.failure, store.hosts.isEmpty {
                ContentUnavailableView {
                    Label("Can't Read the Hosts", systemImage: "server.rack")
                } description: {
                    Text(failure)
                } actions: {
                    Button("Try Again") { store.send(.task) }
                }
            } else if store.hosts.isEmpty {
                ContentUnavailableView {
                    Label("No Hosts", systemImage: "server.rack")
                } description: {
                    Text(
                        "Run sessions on another machine you reach with SSH. Humata installs itself there and "
                            + "connects when a session needs it.")
                } actions: {
                    Button("Add Host…") { store.send(.addTapped) }
                }
            }
        }
        .toolbar {
            ToolbarItem {
                Button {
                    store.send(.addTapped)
                } label: {
                    Label("Add Host", systemImage: "plus")
                }
                .help("Add a machine you reach with SSH")
            }
        }
        .navigationTitle("Hosts")
        .task { await store.send(.task).finish() }
        .sheet(item: $store.scope(state: \.destination?.add, action: \.destination.add)) {
            AddHostView(store: $0)
        }
        .alert($store.scope(state: \.destination?.alert, action: \.destination.alert))
    }

    @ViewBuilder private var detail: some View {
        if let host = store.selected, let editor = store.scope(state: \.editor, action: \.editor) {
            Form {
                HealthSection(
                    host: host, health: store.health[host.id],
                    check: { store.send(.checkTapped(host.id)) },
                    install: { store.send(.installTapped(host.id)) })
                HostEditorSections(store: editor)
                Section {
                    Button("Remove Host…", role: .destructive) { store.send(.removeTapped(host.id)) }
                }
            }
            .formStyle(.grouped)
            .id(host.id)
        }
    }
}

/// A host in the list: its name, where it is, and whether it answered.
private struct HostRow: View {
    let host: RemoteHost
    let health: HostsFeature.Health?

    var body: some View {
        HStack {
            VStack(alignment: .leading) {
                Text(host.name)
                Text(host.sshDestination ?? "Its own command")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .lineLimit(1)
            Spacer()
            switch health {
            case .checking, .installing:
                ProgressView().controlSize(.small)
                    .accessibilityLabel("Checking")
            case .known(.reachable(let status)) where status.problems.isEmpty:
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(.green)
                    .accessibilityLabel("Reachable")
            case .known(.reachable):
                Image(systemName: "exclamationmark.circle.fill")
                    .foregroundStyle(.orange)
                    .accessibilityLabel("Reachable, needs attention")
            case .known(.unreachable):
                Image(systemName: "wifi.slash")
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Unreachable")
            case nil:
                EmptyView()
            }
        }
    }
}

extension HostStatus {
    /// What keeps its sessions from working, for the person to fix.
    var problems: [String] {
        var problems: [String] = []
        if case .failure(let problem) = copilot {
            problems.append("Copilot can't run there: \(problem.message)")
        } else if !githubToken {
            problems.append(
                "Copilot has no GitHub token there. Add GH_TOKEN below, or run `copilot login` on the host.")
        }
        if let setup, !setup.succeeded { problems.append("Its setup script failed; it runs again on the next check.") }
        return problems
    }
}

/// How the host is, with what to do when it needs the person.
private struct HealthSection: View {
    let host: RemoteHost
    let health: HostsFeature.Health?
    let check: () -> Void
    let install: () -> Void

    var body: some View {
        Section {
            switch health {
            case .checking, nil:
                LabeledContent("Status") { ProgressView().controlSize(.small) }
            case .installing:
                LabeledContent("Status") {
                    HStack {
                        ProgressView().controlSize(.small)
                        Text("Installing Humata…")
                    }
                }
            case .known(.unreachable(let problem)):
                LabeledContent("Status", value: "Unreachable")
                Text(problem)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            case .known(.reachable(let status)):
                ForEach(status.problems, id: \.self) { problem in
                    Label(problem, systemImage: "exclamationmark.triangle.fill")
                        .symbolRenderingMode(.multicolor)
                }
                LabeledContent("Humata", value: status.programVersion)
                LabeledContent("System", value: status.platform)
                LabeledContent(
                    "Free Space",
                    value: Int64(clamping: status.freeDiskBytes).formatted(.byteCount(style: .file)))
                switch status.copilot {
                case .success(let version):
                    LabeledContent("Copilot", value: version)
                case .failure:
                    LabeledContent("Copilot", value: "Not working")
                }
                LabeledContent("GitHub Token", value: status.githubToken ? "Set" : "None")
                switch status.claude {
                case .success(let version):
                    LabeledContent("Claude Code", value: version)
                    LabeledContent(
                        "Anthropic Key", value: status.anthropicKey ? "Set" : "None, unless `claude` is signed in there"
                    )
                case .failure(let problem):
                    // Not a problem unless its sessions use Claude Code.
                    LabeledContent("Claude Code", value: "Not installed")
                        .help(problem.message)
                }
                if let setup = status.setup {
                    LabeledContent("Setup", value: setup.succeeded ? "Ran" : "Failed")
                    if !setup.succeeded, !setup.output.isEmpty {
                        Text(setup.output)
                            .font(.caption.monospaced())
                            .textSelection(.enabled)
                            .lineLimit(12)
                    }
                }
            }
        } header: {
            HStack {
                Text(host.name)
                Spacer()
                if case .known(.unreachable) = health, host.sshDestination != nil {
                    Button("Install Humata", action: install)
                        .help("Copy Humata's program to \(host.name)")
                }
                Button("Check Again", action: check)
                    .disabled(health == .checking || health == .installing)
            }
        }
    }
}

/// The host's agents' variables and its setup script, saved together.
private struct HostEditorSections: View {
    @Bindable var store: StoreOf<HostEditorFeature>

    var body: some View {
        Section {
            ForEach($store.variables) { $variable in
                HStack {
                    if variable.saved {
                        Text(variable.name)
                            .font(.body.monospaced())
                            .frame(width: 180, alignment: .leading)
                    } else {
                        TextField("Name", text: $variable.name, prompt: Text("NAME"))
                            .font(.body.monospaced())
                            .frame(width: 180)
                    }
                    SecureField(
                        "Value", text: $variable.value, prompt: Text(variable.saved ? "Unchanged" : "Value"))
                    Button {
                        store.send(.removeVariableTapped(variable.id))
                    } label: {
                        Label("Remove \(variable.name)", systemImage: "minus.circle")
                            .labelStyle(.iconOnly)
                    }
                    .buttonStyle(.borderless)
                }
                .labelsHidden()
            }
            Button("Add Variable") { store.send(.addVariableTapped) }
            if !store.badNames.isEmpty {
                Text(
                    "\(store.badNames.formatted(.list(type: .and))) can't be a variable's name: use letters, "
                        + "digits and _, not starting with a digit."
                )
                .foregroundStyle(.red)
            }
        } header: {
            Text("Agents' Variables")
        } footer: {
            Text(
                "Set for its agents and its setup script, such as GH_TOKEN for Copilot. Humata keeps them on "
                    + "this Mac and sends them when it connects; they're never written on the host."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
        }
        Section {
            TextEditor(text: $store.setupScript)
                .font(.body.monospaced())
                .frame(minHeight: 100)
            HStack {
                Spacer()
                Button("Save") { store.send(.saveTapped) }
                    .keyboardShortcut("s")
                    .disabled(!store.canSave)
            }
        } header: {
            Text("Setup Script")
        } footer: {
            Text(
                "Run with sh in the home folder on the host, once each time it changes, and again until it "
                    + "succeeds; install the tools its agents need here."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
        }
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}

/// Adding a host by its SSH destination.
struct AddHostView: View {
    @Bindable var store: StoreOf<AddHostFeature>
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Form {
            Section {
                if !store.sshHosts.isEmpty {
                    Picker(
                        "From SSH Config",
                        selection: Binding(
                            get: { store.sshHosts.contains(store.destination) ? store.destination : "" },
                            set: { store.send(.sshHostPicked($0)) })
                    ) {
                        Text("Other").tag("")
                        ForEach(store.sshHosts, id: \.self) { Text($0).tag($0) }
                    }
                }
                TextField("Destination", text: $store.destination, prompt: Text("user@machine"))
                TextField("Name", text: $store.name, prompt: Text("What Humata calls it"))
            } header: {
                Text("Add Host")
            } footer: {
                Text(
                    "Humata connects with ssh, so the host must take this Mac's key without a password prompt. "
                        + "It keeps its program and sessions' code in ~/.humata there."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .frame(minWidth: 420, minHeight: 240)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Add") { store.send(.addTapped) }
                    .disabled(!store.canAdd)
            }
        }
        .task { await store.send(.task).finish() }
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}
