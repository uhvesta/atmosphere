import ComposableArchitecture
import Foundation
import LifrAPI

/// A session kept in memory, for tests and previews. `request` plays an
/// unknown app launching the CLI.
package final class InMemorySession: Sendable {
    private struct State {
        var apps: [ConnectedApp]
        var pending: [UUID: ApprovalRequest] = [:]
        var installation = CLIInstallation.notInstalled
        var listeners: [UUID: AsyncStream<SessionEvent>.Continuation] = [:]
    }

    private let state: LockIsolated<State>
    /// Where `installCLI` links, or the reason it fails.
    private let installResult: Result<String, SessionError>

    package init(
        apps: [ConnectedApp] = [], installResult: Result<String, SessionError> = .success("/usr/local/bin/lifr")
    ) {
        state = LockIsolated(State(apps: apps))
        self.installResult = installResult
    }

    /// The apps as they are now.
    package var apps: [ConnectedApp] { state.value.apps }

    /// An unknown app asks for approval.
    package func request(_ request: ApprovalRequest) {
        state.withValue { $0.pending[request.id] = request }
        send(.approvalRequested(request))
    }

    /// The app that asked stops waiting.
    package func withdraw(_ id: UUID) {
        state.withValue { $0.pending[id] = nil }
        send(.approvalWithdrawn(id))
    }

    /// A connected app made a request at `date`.
    package func use(_ app: String, at date: Date, connected: Bool = true) {
        state.withValue { state in
            guard let index = state.apps.firstIndex(where: { $0.id == app }) else { return }
            state.apps[index].lastUsedAt = date
            state.apps[index].isConnected = connected
        }
        send(.appsChanged(sorted()))
    }

    package var client: SessionClient {
        SessionClient(
            apps: { [self] in sorted() },
            events: { [self] in
                let (stream, continuation) = AsyncStream<SessionEvent>.makeStream()
                let id = UUID()
                state.withValue { $0.listeners[id] = continuation }
                continuation.onTermination = { [self] _ in state.withValue { $0.listeners[id] = nil } }
                return stream
            },
            approve: { [self] id, grant in
                try state.withValue { state in
                    guard let request = state.pending.removeValue(forKey: id) else {
                        throw SessionError.unknownRequest(id)
                    }
                    state.apps.removeAll { $0.id == request.identity }
                    state.apps.append(ConnectedApp(id: request.identity, grant: grant, isConnected: true))
                }
                send(.appsChanged(sorted()))
            },
            deny: { [self] id in
                try state.withValue { state in
                    guard state.pending.removeValue(forKey: id) != nil else { throw SessionError.unknownRequest(id) }
                }
            },
            update: { [self] app, grant in
                try state.withValue { state in
                    guard let index = state.apps.firstIndex(where: { $0.id == app }) else {
                        throw SessionError.unknownApp(app)
                    }
                    state.apps[index].grant = grant
                }
                send(.appsChanged(sorted()))
            },
            revoke: { [self] app in
                try state.withValue { state in
                    guard state.apps.contains(where: { $0.id == app }) else { throw SessionError.unknownApp(app) }
                    state.apps.removeAll { $0.id == app }
                }
                send(.appsChanged(sorted()))
            },
            cliInstallation: { [self] in state.value.installation },
            installCLI: { [self] in
                let path = try installResult.get()
                state.withValue { $0.installation = .installed(path: path) }
                return .installed(path: path)
            }
        )
    }

    private func sorted() -> [ConnectedApp] {
        state.value.apps.sorted {
            ($0.lastUsedAt ?? .distantPast, $1.name) > ($1.lastUsedAt ?? .distantPast, $0.name)
        }
    }

    private func send(_ event: SessionEvent) {
        for listener in state.value.listeners.values { listener.yield(event) }
    }
}
