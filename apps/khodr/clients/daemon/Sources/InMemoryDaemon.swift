import ComposableArchitecture
import Foundation

/// A daemon kept in memory, for tests and previews. Each message is answered
/// at once with `reply(text)`. Ids count up (`s1`, `e1`, ...) and times from
/// `start`, so tests see the same values every run.
package final class InMemoryDaemon: Sendable {
    private struct State {
        var sessions: [Session] = []
        var histories: [String: [SessionEvent]] = [:]
        var watchers: [String: [UUID: AsyncThrowingStream<SessionEvent, any Error>.Continuation]] = [:]
        var nextSession = 1
        var nextEvent = 1
    }

    private let state = LockIsolated(State())
    private let reply: @Sendable (String) -> String
    private let start: Date

    package init(
        start: Date = Date(timeIntervalSince1970: 1_794_000_000),
        reply: @escaping @Sendable (String) -> String = { "You said: \($0)" }
    ) {
        self.start = start
        self.reply = reply
    }

    /// The sessions as they are now.
    package var sessions: [Session] { state.value.sessions }

    package func history(_ session: String) -> [SessionEvent] {
        state.value.histories[session] ?? []
    }

    package var client: DaemonClient {
        DaemonClient(
            sessions: { self.state.value.sessions },
            start: { request in self.startSession(request) },
            send: { session, text, _ in try self.play(session, text) },
            approve: { session, request, _ in
                try self.known(session)
                throw DaemonError.refused(
                    .unknownPermissionRequest, message: "Session \(session) isn't waiting on \(request).")
            },
            abort: { session in try self.known(session) },
            fork: { session, before, name in try self.fork(session, before: before, name: name) },
            history: { session in
                try self.known(session)
                return self.history(session)
            },
            watch: { session in self.watch(session) }
        )
    }

    private func known(_ session: String) throws {
        guard state.value.histories[session] != nil else {
            throw DaemonError.refused(.unknownSession, message: "There is no session \(session).")
        }
    }

    private func startSession(_ request: StartRequest) -> Session {
        state.withValue { state in
            let session = Session(
                id: "s\(state.nextSession)", name: request.name, backend: request.backend,
                workingDirectory: request.workingDirectory, model: request.model, host: request.host)
            state.nextSession += 1
            state.sessions.append(session)
            state.histories[session.id] = []
            return session
        }
    }

    private func play(_ session: String, _ text: String) throws {
        try known(session)
        let kinds: [SessionEvent.Kind] = [
            .userMessage(text), .turnStarted, .assistantMessage(reply(text)), .turnEnded(aborted: false), .idle,
        ]
        for kind in kinds {
            append(kind, to: session)
        }
    }

    private func append(_ kind: SessionEvent.Kind, to session: String) {
        let watchers = state.withValue { state in
            let event = SessionEvent(id: "e\(state.nextEvent)", time: start + Double(state.nextEvent), kind: kind)
            state.nextEvent += 1
            state.histories[session, default: []].append(event)
            return (event, Array((state.watchers[session] ?? [:]).values))
        }
        for watcher in watchers.1 {
            watcher.yield(watchers.0)
        }
    }

    private func fork(_ session: String, before: String?, name: String?) throws -> Session {
        try known(session)
        return try state.withValue { state in
            guard let parent = state.sessions.first(where: { $0.id == session }) else {
                throw DaemonError.refused(.unknownSession, message: "There is no session \(session).")
            }
            var history = state.histories[session] ?? []
            if let before {
                guard let index = history.firstIndex(where: { $0.id == before }) else {
                    throw DaemonError.refused(.unknownEvent, message: "Session \(session) has no event \(before).")
                }
                history = Array(history[..<index])
            }
            var fork = parent
            fork.id = "s\(state.nextSession)"
            fork.name = name ?? parent.name
            fork.state = .idle
            fork.parentID = parent.id
            fork.forkedBeforeEventID = before
            state.nextSession += 1
            state.sessions.append(fork)
            state.histories[fork.id] = history
            return fork
        }
    }

    private func watch(_ session: String) -> AsyncThrowingStream<SessionEvent, any Error> {
        AsyncThrowingStream { continuation in
            let id = UUID()
            let known = state.withValue { state -> Bool in
                guard let history = state.histories[session] else { return false }
                for event in history {
                    continuation.yield(event)
                }
                state.watchers[session, default: [:]][id] = continuation
                return true
            }
            guard known else {
                continuation.finish(
                    throwing: DaemonError.refused(.unknownSession, message: "There is no session \(session)."))
                return
            }
            continuation.onTermination = { _ in
                self.state.withValue { $0.watchers[session]?[id] = nil }
            }
        }
    }
}
