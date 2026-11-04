import ComposableArchitecture
import LifrAPI
import LifrCore
import LifrSessionClient
import SwiftUI

package struct SettingsView: View {
    @Bindable package var store: StoreOf<SettingsFeature>

    package init(store: StoreOf<SettingsFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                Stepper(
                    "In Progress for \(store.procrastinatedAfterDays) days",
                    value: Binding(
                        get: { store.procrastinatedAfterDays }, set: { store.send(.procrastinatedDaysChanged($0)) }),
                    in: 1...90)
            } header: {
                Text("Procrastinated")
            } footer: {
                Text("An In Progress task with no activity for longer than this counts as Procrastinated.")
            }
            Section {
                Button("Activity") { store.send(.activityTapped) }
            } footer: {
                Text("Every change, who made it, and from where. Any of them can be reverted.")
            }
            Section {
                Toggle(
                    "Sync conversations",
                    isOn: Binding(
                        get: { store.syncsConversations }, set: { store.send(.syncConversationsToggled($0)) }))
            } header: {
                Text("Assistant")
            } footer: {
                Text(
                    "Keeps conversations with the assistant on your other devices through iCloud, encrypted end to "
                        + "end. Turning it off keeps a copy here; copies in iCloud stay until you delete them.")
            }
            if store.showsAgents {
                avenueAccess
                connectedApps
                commandLine
            }
        }
        .navigationTitle("Settings")
        .navigationDestination(item: $store.scope(state: \.activity, action: \.activity)) { store in
            ActivityView(store: store)
        }
        .task { await store.send(.task).finish() }
        .confirmationDialog($store.scope(state: \.revoke, action: \.revoke))
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    private var avenueAccess: some View {
        Section {
            ForEach(store.avenues) { avenue in
                Picker(
                    avenue.name,
                    selection: Binding(
                        get: { avenue.access }, set: { store.send(.agentAccessPicked(avenue.id, $0)) })
                ) {
                    ForEach(AgentAccess.allCases, id: \.self) { Text($0.title).tag($0) }
                }
            }
        } header: {
            Text("Agent access")
        } footer: {
            Text("Agents never see a Hidden Avenue, whatever their grant says.")
        }
    }

    private var connectedApps: some View {
        Section {
            if store.apps.isEmpty {
                Text("No apps yet. When an agent first runs lifr, Lifr asks you to approve it.")
                    .foregroundStyle(.secondary)
            }
            ForEach(store.apps) { app in
                ConnectedAppRow(app: app, store: store)
            }
        } header: {
            Text("Connected apps")
        }
    }

    private var commandLine: some View {
        Section {
            switch store.cli {
            case .notInstalled:
                Button("Install lifr") { store.send(.installCLITapped) }
            case .installed(let path):
                LabeledContent("Installed at", value: path)
            }
            #if os(macOS)
                Toggle(
                    "Launch at login",
                    isOn: Binding(
                        get: { store.launchesAtLogin }, set: { store.send(.launchAtLoginToggled($0)) }))
            #endif
        } header: {
            Text("Command line")
        } footer: {
            #if os(macOS)
                Text(
                    "Then connect Claude Code with: claude mcp add lifr -- lifr mcp. Launch at login keeps Lifr in the "
                        + "menu bar, so lifr never waits for it to start.")
            #else
                Text("Then connect Claude Code with: claude mcp add lifr -- lifr mcp")
            #endif
        }
    }
}

private struct ConnectedAppRow: View {
    let app: ConnectedApp
    let store: StoreOf<SettingsFeature>

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(app.name).font(.headline)
                if app.isConnected {
                    Image(systemName: "circle.fill")
                        .font(.caption2)
                        .foregroundStyle(.green)
                        .accessibilityLabel("Connected")
                }
                Spacer()
                Button("Revoke", role: .destructive) { store.send(.revokeTapped(app.id)) }
            }
            Picker(
                "Access",
                selection: Binding(get: { app.canWrite }, set: { store.send(.writePicked(app.id, $0)) })
            ) {
                Text("Read only").tag(false)
                Text("Read and write").tag(true)
            }
            HStack {
                Group {
                    if let lastUsed = app.lastUsedAt {
                        Text("Last used \(lastUsed, format: .dateTime.month().day().hour().minute())")
                    } else {
                        Text("Not used yet")
                    }
                    Text("Expires \(app.grant.expiresAt, format: .dateTime.month().day().year())")
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                Spacer()
                Button("Renew") { store.send(.renewTapped(app.id)) }
                    .font(.caption)
            }
        }
        .buttonStyle(.borderless)
    }
}

/// The approval sheet.
package struct ApprovalView: View {
    @Bindable package var store: StoreOf<ApprovalFeature>

    package init(store: StoreOf<ApprovalFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                Text("\(store.request.client) wants to use your tasks through Lifr.")
                    .font(.headline)
                LabeledContent("Signed as", value: store.request.identity)
            } footer: {
                if store.request.isInterpreter {
                    Text(
                        "\(store.request.client) runs scripts, and every script it runs would share this access. "
                            + "Approve it only for a short time."
                    )
                    .foregroundStyle(.orange)
                }
            }
            Section {
                Toggle("Make changes", isOn: $store.canWrite)
                if store.canWrite {
                    Toggle("Ask me before each change", isOn: $store.confirmWrites)
                }
            } header: {
                Text("Access")
            } footer: {
                Text(
                    "It can always read. Agents can never empty the trash, change Avenues or Settings, or approve apps."
                )
            }
            Section {
                Toggle("All Avenues", isOn: $store.allAvenues)
                if !store.allAvenues {
                    ForEach(store.avenues) { avenue in
                        Toggle(avenue.name, isOn: $store.avenueIDs.contains(avenue.id))
                    }
                }
            } header: {
                Text("Avenues")
            } footer: {
                Text("Hidden Avenues aren't listed; agents never see them.")
            }
            Section {
                Picker("Expires after", selection: $store.expiry) {
                    ForEach(ApprovalFeature.Expiry.allCases, id: \.self) { Text($0.title).tag($0) }
                }
            }
            if let error = store.error {
                Section { Text(error).foregroundStyle(.red) }
            }
        }
        .navigationTitle("Approve \(store.request.client)?")
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Deny") { store.send(.denyTapped) }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Approve") { store.send(.approveTapped) }.disabled(!store.canApprove)
            }
        }
    }
}

extension Set {
    fileprivate subscript(contains element: Element) -> Bool {
        get { contains(element) }
        set {
            if newValue { insert(element) } else { remove(element) }
        }
    }
}

extension Binding {
    fileprivate func contains<Element: Hashable>(_ element: Element) -> Binding<Bool> where Value == Set<Element> {
        self[contains: element]
    }
}

#Preview {
    NavigationStack {
        SettingsView(store: Store(initialState: SettingsFeature.State(showsAgents: true)) { SettingsFeature() })
    }
}
