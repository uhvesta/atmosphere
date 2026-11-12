import AppKit
import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// The host's forwarded ports, each opened in the browser, and a field to
/// forward another.
struct PortsView: View {
    @Bindable var store: StoreOf<PortsFeature>

    var body: some View {
        Form {
            Section {
                HStack {
                    TextField("Port", text: $store.port, prompt: Text("3000"))
                        .frame(width: 120)
                        .onSubmit { store.send(.forwardTapped) }
                    Button("Forward") { store.send(.forwardTapped) }
                        .disabled(store.portNumber == nil || store.forwarding)
                    if store.forwarding { ProgressView().controlSize(.small) }
                }
            } header: {
                Text("Forward a port")
            } footer: {
                Text(
                    "Listens on 127.0.0.1 on this Mac only, and carries each connection over the host's SSH "
                        + "connection; nothing else can reach it."
                )
                .foregroundStyle(.secondary)
            }
            if let failure = store.failure {
                Label(failure, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.red)
            }
            Section("Forwarded") {
                if store.forwards.isEmpty {
                    Text("No ports are forwarded from this host.")
                        .foregroundStyle(.secondary)
                }
                ForEach(store.forwards) { forward in
                    row(forward)
                }
            }
        }
        .formStyle(.grouped)
        .task { await store.send(.task).finish() }
    }

    private func row(_ forward: Forward) -> some View {
        HStack {
            VStack(alignment: .leading) {
                Text(verbatim: "Port \(forward.port)")
                Text(verbatim: "127.0.0.1:\(forward.localPort)")
                    .font(.callout.monospaced())
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            Spacer()
            if let url = forward.url {
                Button("Open in Browser") { NSWorkspace.shared.open(url) }
            }
            Button("Stop") { store.send(.stopTapped(forward.id)) }
        }
    }
}
