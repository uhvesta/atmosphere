import ComposableArchitecture
import Foundation
import HumataDaemonClient
import SwiftUI

/// The sheet that brings a Claude Code or Copilot session started outside
/// Humata in, with its history, to carry on in the folder it worked in.
@Reducer
package struct ImportSessionFeature {
    @ObservableState
    package struct State: Equatable {
        /// The projects it could go in.
        package var projects: [Project]
        /// The one it goes in; nil for a chat.
        package var projectID: Project.ID?
        /// The hosts to look on besides this Mac.
        package var hosts: [RemoteHost] = []
        /// Where to look; nil for this Mac.
        package var host: RemoteHost.ID?
        /// The host's sessions, newest first; nil while they're read.
        package var sessions: [ImportableSession]?
        /// Why they couldn't be read.
        package var loadError: String?
        package var selection: ImportableSession.ID?
        package var importing = false
        @Presents package var alert: AlertState<Never>?

        package init(projects: [Project]) {
            self.projects = projects
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// The sheet appeared: reads the hosts, then this Mac's sessions.
        case task
        case hostsLoaded([RemoteHost])
        case loaded(host: RemoteHost.ID?, Result<[ImportableSession], any Error>)
        case importTapped
        case failed(String)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case imported(Session)
        }
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding(\.host):
                return load(&state)

            case .binding, .alert, .delegate:
                return .none

            case .task:
                return .merge(
                    .run { send in
                        // Without them, it looks on this Mac only.
                        await send(.hostsLoaded((try? await daemon.hosts()) ?? []))
                    },
                    load(&state))

            case .hostsLoaded(let hosts):
                state.hosts = hosts
                return .none

            case .loaded(let host, let result):
                // An answer for a host no longer picked is stale.
                guard host == state.host else { return .none }
                switch result {
                case .success(let sessions):
                    state.sessions = sessions
                case .failure(let error):
                    state.sessions = []
                    state.loadError = error.localizedDescription
                }
                return .none

            case .importTapped:
                guard !state.importing, let id = state.selection,
                    let session = state.sessions?.first(where: { $0.id == id })
                else { return .none }
                state.importing = true
                let host = state.host
                let projectID = state.projectID
                return .run { send in
                    await send(.delegate(.imported(try await daemon.importSession(session, host, projectID))))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .failed(let message):
                state.importing = false
                state.alert = AlertState {
                    TextState("Couldn't import the session")
                } message: {
                    TextState(message)
                }
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }

    private func load(_ state: inout State) -> Effect<Action> {
        state.sessions = nil
        state.loadError = nil
        state.selection = nil
        let host = state.host
        return .run { send in
            await send(.loaded(host: host, Result { try await daemon.importable(host) }))
        }
    }
}

package struct ImportSessionView: View {
    @Bindable var store: StoreOf<ImportSessionFeature>
    @Environment(\.dismiss) private var dismiss

    package init(store: StoreOf<ImportSessionFeature>) {
        self.store = store
    }

    package var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Import a Session")
                .font(.headline)
            Text(
                "Claude Code and Copilot sessions started outside Humata. "
                    + "The imported one carries on the same conversation, in the folder it worked in."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            Form {
                if !store.hosts.isEmpty {
                    Picker("On", selection: $store.host) {
                        Text("This Mac").tag(RemoteHost.ID?.none)
                        ForEach(store.hosts) { host in
                            Text(host.name).tag(RemoteHost.ID?.some(host.id))
                        }
                    }
                }
                Picker("Project", selection: $store.projectID) {
                    Text("None, a Chat").tag(Project.ID?.none)
                    ForEach(store.projects) { project in
                        Text(project.name).tag(Project.ID?.some(project.id))
                    }
                }
            }
            sessions
        }
        .padding()
        .frame(minWidth: 520, minHeight: 380)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            if store.importing {
                ToolbarItem(placement: .status) {
                    HStack(spacing: 6) {
                        ProgressView().controlSize(.small)
                        Text("Importing…")
                            .foregroundStyle(.secondary)
                    }
                }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Import") { store.send(.importTapped) }
                    .disabled(store.selection == nil || store.importing)
            }
        }
        .alert($store.scope(state: \.alert, action: \.alert))
        .task { await store.send(.task).finish() }
    }

    @ViewBuilder private var sessions: some View {
        if let sessions = store.sessions {
            if let error = store.loadError {
                ContentUnavailableView(
                    "Couldn't read the sessions", systemImage: "exclamationmark.triangle", description: Text(error))
            } else if sessions.isEmpty {
                ContentUnavailableView(
                    "Nothing to import", systemImage: "tray",
                    description: Text("There are no Claude Code or Copilot sessions here that Humata hasn't imported."))
            } else {
                List(sessions, selection: $store.selection) { session in
                    ImportableRow(session: session)
                }
            }
        } else {
            ProgressView()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

/// One session to import: what it was about, its agent, folder and when.
private struct ImportableRow: View {
    let session: ImportableSession

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(session.summary)
                .lineLimit(1)
            HStack(spacing: 6) {
                Text(session.backend == .claude ? "Claude Code" : "Copilot")
                Text((session.workingDirectory as NSString).abbreviatingWithTildeInPath)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer()
                Text(session.updated, format: .dateTime.month().day().hour().minute())
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }
}
