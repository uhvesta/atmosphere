import ComposableArchitecture
import HumataBrowser
import HumataDaemonClient
import HumataTerminal
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
                Section {
                    Label("This Mac", systemImage: "laptopcomputer")
                        .tag(store.coordinator.id)
                }
                Section("Remote") {
                    ForEach(store.hosts) { host in
                        HostRow(host: host, health: store.health[host.id])
                            .tag(host.id)
                    }
                    if store.hosts.isEmpty {
                        Button("Add Host…") { store.send(.addTapped) }
                            .buttonStyle(.link)
                            .help("Run sessions on another machine you reach with SSH")
                    }
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
        if store.selected != nil {
            VStack(spacing: 0) {
                Picker("Show", selection: $store.pane.sending(\.paneSelected)) {
                    Text("Settings").tag(HostsFeature.Pane.settings)
                    Text("Terminals").tag(HostsFeature.Pane.terminals)
                    Text("Files").tag(HostsFeature.Pane.files)
                    Text("Ports").tag(HostsFeature.Pane.ports)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()
                .padding(8)
                .help("The host's settings, the shells running on it, its files, or the ports forwarded from it")
                if store.pane == .terminals, let terminals = store.scope(state: \.terminals, action: \.terminals) {
                    TerminalsView(store: terminals)
                        .id(store.selection)
                } else if store.pane == .files, let files = store.scope(state: \.files, action: \.files) {
                    FileBrowserView(store: files)
                        .id(store.selection)
                } else if store.pane == .ports, let ports = store.scope(state: \.ports, action: \.ports) {
                    PortsView(store: ports)
                        .id(store.selection)
                } else {
                    settings
                }
            }
        }
    }

    @ViewBuilder private var settings: some View {
        if let host = store.selected, host.isCoordinator,
            let editor = store.scope(state: \.editor, action: \.editor)
        {
            Form {
                if let links = store.commandLinks {
                    CommandsSection(
                        links: links, linking: store.linking,
                        link: { store.send(.linkCommandsTapped) },
                        restart: { store.send(.restartDaemonTapped) })
                }
                HostEditorSections(store: editor)
                SkippedSection(
                    skipped: store.skipped, failure: store.skippedFailure, thisMac: true,
                    remove: { store.send(.removeSkippedTapped($0)) },
                    retry: { store.send(.skippedRetryTapped) },
                    restart: { store.send(.restartDaemonTapped) })
            }
            .formStyle(.grouped)
            .id(host.id)
        } else if let host = store.selected, let editor = store.scope(state: \.editor, action: \.editor) {
            Form {
                HealthSection(
                    host: host, health: store.health[host.id],
                    check: { store.send(.checkTapped(host.id)) },
                    install: { store.send(.installTapped(host.id)) })
                RecoverySection(
                    recoveries: store.recoveries, failure: store.recoveriesFailure, restoring: store.restoring,
                    others: store.hosts.ids.filter { $0 != host.id },
                    restore: { store.send(.restoreTapped($0)) },
                    restoreAll: { store.send(.restoreAllTapped) },
                    move: { store.send(.moveTapped($0, to: $1)) },
                    retry: { store.send(.recoveriesRetryTapped) })
                HostEditorSections(store: editor)
                SkippedSection(
                    skipped: store.skipped, failure: store.skippedFailure, thisMac: false,
                    remove: { store.send(.removeSkippedTapped($0)) },
                    retry: { store.send(.skippedRetryTapped) },
                    restart: {})
                Section {
                    Button("Remove Host…", role: .destructive) { store.send(.removeTapped(host.id)) }
                }
            }
            .formStyle(.grouped)
            .id(host.id)
        }
    }
}

/// This Mac's `humata` and `humatad` in `~/.local/bin`, so a terminal finds
/// them: how each is linked, a button to link them, and what to add to the
/// shell's startup file when that folder isn't on its PATH, which Humata
/// never changes itself.
private struct CommandsSection: View {
    let links: CommandLinks
    let linking: Bool
    let link: () -> Void
    /// Asks before restarting the daemon, which reads the shell again.
    let restart: () -> Void

    var body: some View {
        Section {
            if let problem = links.environmentProblem {
                Label(HostStatus.environment(problem, thisMac: true), systemImage: "exclamationmark.triangle.fill")
                    .symbolRenderingMode(.multicolor)
                Button("Restart Humata's Daemon…", action: restart)
                    .help("Start the daemon anew so it reads your login shell again; turns in progress stop")
            }
            ForEach(links.links, id: \.name) { command in
                LabeledContent {
                    Text(Self.describe(command.state))
                        .foregroundStyle(command.state == .linked ? .primary : .secondary)
                } label: {
                    Text(command.name).monospaced()
                }
            }
            if links.linkable {
                Button(linking ? "Linking…" : "Link Commands", action: link)
                    .disabled(linking)
                    .help("Link humata and humatad into \(links.folder); nothing else there is replaced")
            }
            if !links.onPath {
                VStack(alignment: .leading, spacing: 4) {
                    Text(
                        "\(links.folder) isn't on your shell's PATH. Add this line to ~/.zprofile, then open "
                            + "a new terminal:")
                    Text("export PATH=\"$HOME/.local/bin:$PATH\"")
                        .monospaced()
                        .textSelection(.enabled)
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        } header: {
            Text("Commands")
        } footer: {
            Text("Linked into \(links.folder), the humata and humatad here run in any terminal on this Mac.")
        }
    }

    private static func describe(_ state: CommandLinks.Link.State) -> String {
        switch state {
        case .missing: "Not linked"
        case .linked: "Linked"
        case .stale: "Linked to another copy of Humata"
        case .other: "Another program of this name is there; it's left alone"
        }
    }
}

/// The Mac's copies of the host's sessions: how far each is backed up, and
/// for those the host lost, a Restore button; any can move to another host
/// or this Mac. None shows nothing, unless they couldn't be read, which
/// it says with a way to read them again.
private struct RecoverySection: View {
    let recoveries: IdentifiedArrayOf<Recovery>
    /// Why they couldn't be read; nil when they were.
    let failure: String?
    let restoring: Set<Recovery.ID>
    /// The other hosts, to move a session to.
    let others: [RemoteHost.ID]
    let restore: (Recovery.ID) -> Void
    let restoreAll: () -> Void
    let move: (Recovery.ID, RemoteHost.ID?) -> Void
    let retry: () -> Void

    var body: some View {
        if !recoveries.isEmpty || failure != nil {
            Section {
                if let failure {
                    UnreadRow(
                        what: "Couldn't read this Mac's copies of the host's sessions", why: failure, retry: retry)
                }
                ForEach(recoveries) { recovery in
                    HStack(alignment: .firstTextBaseline) {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(recovery.name.isEmpty ? recovery.id : recovery.name)
                                .lineLimit(1)
                                .truncationMode(.middle)
                            Text(status(of: recovery))
                                .font(.caption)
                                .foregroundStyle(
                                    recovery.onHost && recovery.problem.isEmpty ? Color.secondary : Color.orange)
                        }
                        Spacer()
                        if restoring.contains(recovery.id) {
                            ProgressView().controlSize(.small)
                        } else if recovery.hasCopy {
                            if !recovery.onHost {
                                Button("Restore") { restore(recovery.id) }
                            }
                            Menu("Move") {
                                Button("This Mac") { move(recovery.id, nil) }
                                ForEach(others, id: \.self) { host in
                                    Button(host) { move(recovery.id, host) }
                                }
                            }
                            .menuStyle(.borderlessButton)
                            .fixedSize()
                            .accessibilityLabel("Move \(recovery.name.isEmpty ? recovery.id : recovery.name)")
                        }
                    }
                }
                if recoveries.filter({ !$0.onHost }).count > 1 {
                    Button("Restore All Lost Sessions") { restoreAll() }
                        .disabled(recoveries.allSatisfy { $0.onHost || restoring.contains($0.id) })
                }
            } header: {
                Text("Recovery Copies")
            } footer: {
                Text(
                    "After each turn this Mac keeps a copy of every session here: its history, the agent's own "
                        + "state and its code. If the host loses a session, restore it. Moving one carries it on "
                        + "elsewhere from its copy; turns the host ran since the copy aren't brought along.")
            }
        }
    }

    private func status(of recovery: Recovery) -> String {
        guard recovery.problem.isEmpty else {
            let when = recovery.hasCopy ? " · the copy is from turn \(recovery.backedUpTurns)" : ""
            return "Couldn't back up: \(recovery.problem)\(when)"
        }
        let copy =
            recovery.backedUpTurns == recovery.turns
            ? "Backed up after every turn"
            : "Backed up after \(recovery.backedUpTurns) of \(recovery.turns) turns"
        return recovery.onHost ? copy : "Lost by the host · \(copy.lowercased())"
    }
}

/// The host's sessions whose files couldn't be read, with why; none shows
/// nothing, unless the list itself couldn't be read, which it says with a
/// way to read it again. A daemon reads its sessions when it starts, so the
/// way back for one that's fixed is the daemon started anew: a button for
/// This Mac's, and for a host's what stops it there.
private struct SkippedSection: View {
    let skipped: IdentifiedArrayOf<SkippedSession>
    /// Why the list couldn't be read; nil when it was.
    let failure: String?
    /// Whether these are This Mac's, whose daemon Humata can restart.
    let thisMac: Bool
    let remove: (SkippedSession.ID) -> Void
    let retry: () -> Void
    /// Asks before restarting This Mac's daemon; not used for a host.
    let restart: () -> Void

    var body: some View {
        if !skipped.isEmpty || failure != nil {
            Section {
                if let failure {
                    UnreadRow(what: "Couldn't read which sessions couldn't be opened", why: failure, retry: retry)
                }
                ForEach(skipped) { session in
                    HStack(alignment: .firstTextBaseline) {
                        VStack(alignment: .leading, spacing: 4) {
                            Text("\(session.backend.agentName) · \(session.id)")
                                .lineLimit(1)
                                .truncationMode(.middle)
                            Text(session.reason)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .textSelection(.enabled)
                        }
                        Spacer()
                        Button("Remove…", role: .destructive) { remove(session.id) }
                    }
                }
                if thisMac, !skipped.isEmpty {
                    Button("Restart Humata's Daemon…", action: restart)
                        .help("Start the daemon anew so it reads these sessions again; turns in progress stop")
                }
            } header: {
                Text("Sessions That Couldn't Be Opened")
            } footer: {
                Text(
                    thisMac
                        ? "Humata left these as they are. To open one again, fix what's damaged, then restart "
                            + "Humata's daemon with the button above. Or remove it to delete its folder and history."
                        : "Humata left these as they are. To open one again, fix what's damaged there, then stop "
                            + "humatad on the host (for example `pkill -x humatad`; turns in progress there stop) "
                            + "and press Check Again, which starts it anew. Or remove it to delete its folder and "
                            + "history.")
            }
        }
    }
}

/// A list that couldn't be read: what, why, and a way to read it again.
private struct UnreadRow: View {
    let what: String
    let why: String
    let retry: () -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            VStack(alignment: .leading, spacing: 4) {
                Label(what, systemImage: "exclamationmark.triangle.fill")
                    .symbolRenderingMode(.multicolor)
                Text(why)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            Spacer()
            Button("Try Again", action: retry)
        }
    }
}

extension Backend {
    fileprivate var agentName: String {
        switch self {
        case .claude: "Claude Code"
        case .copilot: "Copilot"
        case .local: "Local Model"
        case .scripted: "Scripted"
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
            case .checking:
                ProgressView().controlSize(.small)
                    .accessibilityLabel("Checking")
            case .installing:
                ProgressView().controlSize(.small)
                    .accessibilityLabel("Installing Humata")
            case .installFailed:
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                    .accessibilityLabel("Couldn't install Humata")
            case .known(.reachable(let status)) where !status.needsAttention:
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
    /// Whether something keeps its sessions from working whichever agent
    /// they use: its shell's variables weren't read, or its setup script
    /// failed. An agent that isn't installed or signed in is a row, not this.
    package var needsAttention: Bool { !problems.isEmpty }

    /// What keeps its sessions from working, for the person to fix.
    var problems: [String] {
        var problems: [String] = []
        if let environmentProblem { problems.append(Self.environment(environmentProblem, thisMac: false)) }
        if let setup, !setup.succeeded { problems.append("Its setup script failed; it runs again on the next check.") }
        return problems
    }

    /// What it means that the login shell's variables weren't read, and
    /// what reads them again: they're read once, when the daemon starts.
    static func environment(_ problem: String, thisMac: Bool) -> String {
        let found =
            "Agents run without your shell's PATH and exports, so they may not find programs or keys: "
            + "\(problem). "
        if thisMac { return found + "Fix it, then restart Humata's daemon with the button below." }
        return found
            + "Fix it there, then stop humatad on the host (for example `pkill -x humatad`; turns in progress "
            + "there stop) and press Check Again, which starts it anew."
    }
}

/// How the host is, with what to do when it needs the person.
struct HealthSection: View {
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
            case .installFailed(let problem):
                LabeledContent("Status", value: "Couldn't Install Humata")
                Text(problem)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            case .known(.unreachable(let problem)):
                LabeledContent("Status", value: "Unreachable")
                Text(problem)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                if host.sshDestination != nil {
                    Text("If Humata isn't installed on \(host.name) yet, Install Humata above copies it there.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
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
                case .failure(let problem):
                    // Not a problem unless its sessions use Copilot.
                    LabeledContent("Copilot", value: "Not installed")
                        .help(problem.message)
                }
                LabeledContent(
                    "GitHub Token", value: status.githubToken ? "Set" : "None, unless Copilot is signed in there")
                switch status.claude {
                case .success(let version):
                    LabeledContent("Claude Code", value: version)
                    LabeledContent(
                        "Anthropic Key",
                        value: status.anthropicKey ? "Set" : "None, unless Claude Code is signed in there"
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
                if case .installFailed = health, host.sshDestination != nil {
                    Button("Try the Install Again", action: install)
                        .help("Copy Humata's program to \(host.name) again")
                } else if case .known(.unreachable) = health, host.sshDestination != nil {
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
struct HostEditorSections: View {
    @Bindable var store: StoreOf<HostEditorFeature>

    var body: some View {
        Group {
            variables
            providers
            if !store.host.isCoordinator {
                setupScript
            }
        }
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    /// The providers its sessions can pay for with a key of the person's
    /// own, each naming the variable above that holds the key.
    @ViewBuilder private var providers: some View {
        Section {
            ForEach($store.providers) { $provider in
                VStack(alignment: .leading, spacing: 6) {
                    HStack {
                        TextField("Name", text: $provider.name, prompt: Text(provider.kind.title))
                        Text(provider.kind.title)
                            .foregroundStyle(.secondary)
                        Button {
                            store.send(.removeProviderTapped(provider.id))
                        } label: {
                            Label("Remove \(provider.name)", systemImage: "minus.circle")
                                .labelStyle(.iconOnly)
                        }
                        .buttonStyle(.borderless)
                    }
                    TextField("Base URL", text: $provider.baseURL, prompt: Text(provider.kind.baseURL))
                    TextField("Key Variable", text: $provider.keyVariable, prompt: Text(provider.kind.keyVariable))
                        .font(.body.monospaced())
                    if provider.takesWireAPI {
                        Picker("Copilot Asks With", selection: $provider.wireAPI) {
                            Text("Chat Completions").tag(HostProvider.WireAPI.completions)
                            Text("Responses").tag(HostProvider.WireAPI.responses)
                        }
                    }
                    if let problem = store.state.problem(with: provider) {
                        Text(problem)
                            .font(.caption)
                            .foregroundStyle(.red)
                    } else if !store.state.keyIsSet(for: provider), !store.state.offersAddKeyVariable(for: provider) {
                        Text("Type the key in the \(provider.keyVariable) row under Variables, then save.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    } else if !store.state.keyIsSet(for: provider) {
                        HStack {
                            Text(
                                "\(provider.keyVariable) isn't set, so new sessions aren't offered it. Add it under "
                                    + "Variables with the key as its value."
                            )
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            Spacer()
                            Button("Add \(provider.keyVariable)") { store.send(.addKeyVariableTapped(provider.id)) }
                        }
                    }
                }
                .padding(.vertical, 2)
            }
            HStack {
                Menu("Add Provider") {
                    ForEach(HostProvider.Kind.allCases, id: \.self) { kind in
                        Button(kind.title) { store.send(.addProviderTapped(kind)) }
                    }
                }
                .fixedSize()
                if store.host.isCoordinator {
                    // This Mac has no setup script to save them with.
                    Spacer()
                    saveButton
                }
            }
        } header: {
            Text("Model Providers")
        } footer: {
            Text(
                "Pay for a session's model with a key of your own: Claude Code on the Anthropic API, a local "
                    + "model on an OpenAI-compatible server or OpenCode Go, Copilot on any. Each names the variable "
                    + "that holds its key; the key itself stays among the variables, never here."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var variables: some View {
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
            Text(store.host.isCoordinator ? "This Mac's Agents' Variables" : "Agents' Variables")
        } footer: {
            Text(
                store.host.isCoordinator
                    ? "Set for the agents on this Mac on top of your login shell's, such as GH_TOKEN for Copilot. "
                        + "Humata keeps them in its hosts file, which only you can read."
                    : "Set for its agents and its setup script, such as GH_TOKEN for Copilot. Humata keeps them on "
                        + "this Mac and sends them when it connects; they're never written on the host."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }

    private var saveButton: some View {
        Button("Save") { store.send(.saveTapped) }
            .keyboardShortcut("s")
            .disabled(!store.canSave)
    }

    @ViewBuilder private var setupScript: some View {
        Section {
            TextEditor(text: $store.setupScript)
                .font(.body.monospaced())
                .frame(minHeight: 100)
            HStack {
                Spacer()
                saveButton
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
