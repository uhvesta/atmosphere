import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// Settings > Environments: the machines in ~/.ssh/config, each added or
/// not, beside the selected one's health, variables and setup script.
package struct EnvironmentsView: View {
    @Bindable var store: StoreOf<EnvironmentsFeature>

    package init(store: StoreOf<EnvironmentsFeature>) {
        self.store = store
    }

    package var body: some View {
        HStack(spacing: 0) {
            List(selection: $store.selection.sending(\.selected)) {
                Section {
                    ForEach(store.environments) { environment in
                        EnvironmentRow(
                            environment: environment,
                            health: environment.host.flatMap { store.health[$0.id] },
                            adding: store.adding.contains(environment.destination),
                            needsUpdate: environment.host.map { store.state.needsUpdate($0.id) } ?? false,
                            add: { store.send(.addTapped(destination: environment.destination)) },
                            update: { environment.host.map { store.send(.updateTapped($0.id)) } }
                        )
                        .tag(environment.host?.id)
                        .selectionDisabled(environment.host == nil)
                    }
                } header: {
                    Text("From ~/.ssh/config")
                } footer: {
                    if store.environments.isEmpty {
                        Text("Hosts you add to ~/.ssh/config show here.")
                    }
                }
                Button("Add Other…") { store.send(.addOtherTapped) }
                    .buttonStyle(.link)
                    .help("Add a machine by user@address")
            }
            .frame(width: 260)
            Divider()
            detail
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .overlay {
            if let failure = store.failure, store.environments.isEmpty {
                ContentUnavailableView {
                    Label("Can't Read the Hosts", systemImage: "server.rack")
                } description: {
                    Text(failure)
                } actions: {
                    Button("Try Again") { store.send(.task) }
                }
            }
        }
        .task { await store.send(.task).finish() }
        .sheet(item: $store.scope(state: \.destination?.addOther, action: \.destination.addOther)) { add in
            AddHostView(store: add)
        }
        .alert($store.scope(state: \.destination?.alert, action: \.destination.alert))
    }

    @ViewBuilder private var detail: some View {
        if let id = store.selection, let host = store.hosts[id: id] {
            Form {
                HealthSection(
                    host: host, health: store.health[id],
                    check: { store.send(.checkTapped(id)) },
                    install: { store.send(.updateTapped(id)) })
                if let editor = store.scope(state: \.editor, action: \.editor) {
                    HostEditorSections(store: editor)
                }
                Section {
                    Button("Remove Host…", role: .destructive) { store.send(.removeTapped(id)) }
                }
            }
            .formStyle(.grouped)
        } else {
            ContentUnavailableView(
                "No Environment Selected", systemImage: "server.rack",
                description: Text("Add a host to run sessions on it. Humata installs its program there over SSH."))
        }
    }
}

/// A machine in the list: its state, and Add or Update when it needs one.
private struct EnvironmentRow: View {
    let environment: EnvironmentsFeature.Environment
    let health: HostsFeature.Health?
    let adding: Bool
    let needsUpdate: Bool
    let add: () -> Void
    let update: () -> Void

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(environment.host?.name ?? environment.destination)
                    .lineLimit(1)
                Text(status)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 8)
            if adding || health == .installing || health == .checking {
                ProgressView().controlSize(.small)
                    // What it waits for, as the status beside it says.
                    .accessibilityLabel(status)
            } else if environment.host == nil {
                Button("Add", action: add)
                    .help("Add \(environment.destination) and install Humata there")
            } else if needsUpdate {
                Button("Update", action: update)
                    .help("Install this app's build of Humata there")
            }
        }
    }

    private var status: String {
        if adding { return "Adding…" }
        guard environment.host != nil else { return "Not added" }
        switch health {
        case .checking, nil: return "Checking…"
        case .installing: return "Installing Humata…"
        case .installFailed: return "Couldn't install Humata"
        case .known(.reachable(let status)): return "Humata \(status.programVersion) · \(status.platform)"
        case .known(.unreachable): return "Unreachable"
        }
    }
}
