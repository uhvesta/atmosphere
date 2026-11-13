import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// Terminals at one place, a session's folder or a host, as tabs: those the
/// host already runs there, opening another, and closing one. Each keeps
/// running on its host while nothing shows it.
@Reducer
package struct TerminalsFeature {
    @ObservableState
    package struct State: Equatable {
        package var place: HostPlace
        package var tabs: IdentifiedArrayOf<TerminalFeature.State> = []
        package var selected: Terminal.ID?
        /// Whether the host's terminals have been read once.
        package var listed = false
        /// Opens one when the host runs none there yet.
        package var opensFirst: Bool
        package var opening = false
        /// Why the terminals couldn't be read, opened or closed, and what to
        /// do about it.
        package var failure: String?

        /// `opensFirst` opens a terminal when there is none, as a session's
        /// tab does; a host's list waits to be asked.
        package init(place: HostPlace, opensFirst: Bool) {
            self.place = place
            self.opensFirst = opensFirst
        }

        package var current: TerminalFeature.State? { selected.flatMap { tabs[id: $0] } }

        /// Whether `terminal` belongs here: a session's own, or any on the
        /// host when this isn't a session's.
        func shows(_ terminal: Terminal) -> Bool {
            if let session = place.sessionID { return terminal.sessionID == session }
            return terminal.host == place.host
        }
    }

    package enum Action {
        /// The tabs appeared: reads the terminals running here.
        case task
        case listed([Terminal])
        case newTapped
        case opened(Terminal)
        case selected(Terminal.ID)
        case closeTapped(Terminal.ID)
        case closed(Terminal.ID)
        case failed(String)
        case terminals(IdentifiedActionOf<TerminalFeature>)
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                let host = state.place.host
                return .run { send in
                    await send(.listed(try await daemon.terminals(host)))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .listed(let terminals):
                let first = !state.listed
                state.listed = true
                state.failure = nil
                for terminal in terminals where state.shows(terminal) && state.tabs[id: terminal.id] == nil {
                    state.tabs.append(TerminalFeature.State(terminal: terminal))
                }
                if state.selected == nil { state.selected = state.tabs.last?.id }
                guard first, state.tabs.isEmpty, state.opensFirst else { return .none }
                return .send(.newTapped)

            case .newTapped:
                guard !state.opening else { return .none }
                state.opening = true
                let place = state.place
                return .run { send in
                    await send(.opened(try await daemon.openTerminal(place, .standard)))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .opened(let terminal):
                state.opening = false
                state.failure = nil
                state.tabs.append(TerminalFeature.State(terminal: terminal))
                state.selected = terminal.id
                return .none

            case .selected(let id):
                state.selected = id
                return .none

            case .closeTapped(let id):
                guard let terminal = state.tabs[id: id]?.terminal else { return .none }
                return .run { send in
                    try await daemon.closeTerminal(terminal)
                    await send(.closed(id))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .closed(let id):
                let index = state.tabs.index(id: id)
                state.tabs.remove(id: id)
                if state.selected == id, let index {
                    state.selected = state.tabs.isEmpty ? nil : state.tabs[min(index, state.tabs.count - 1)].id
                }
                return .none

            case .failed(let message):
                state.opening = false
                state.failure = message
                return .none

            case .terminals:
                return .none
            }
        }
        .forEach(\.tabs, action: \.terminals) {
            TerminalFeature()
        }
    }
}

/// One terminal: what it shows, kept as the bytes it printed, and the keys
/// and sizes sent to it. Attaching replays what it printed so far, so a
/// dropped connection only needs another attach.
@Reducer
package struct TerminalFeature {
    /// How much of what it printed is kept to draw; the host keeps the same.
    static let kept = 256 * 1024

    @ObservableState
    package struct State: Equatable, Identifiable {
        package var terminal: Terminal
        package var screen = Screen()
        /// Whether the stream of its output ended without it exiting.
        package var detached = false
        /// Why typing or attaching failed, and what to do about it.
        package var failure: String?
        /// Keys typed while those before them are on their way, sent next
        /// in one go, so they arrive in the order they were typed.
        var pending = Data()
        var sending = false
        /// Reattaches since output last came, to wait longer each time.
        var retries = 0

        package init(terminal: Terminal) {
            self.terminal = terminal
        }

        package var id: Terminal.ID { terminal.id }
        package var exited: Bool { terminal.exitCode != nil }
    }

    /// The bytes a terminal printed, from `start`: the view draws them from
    /// the beginning whenever `generation` changes or it fell behind `start`,
    /// and otherwise only what's new.
    package struct Screen: Equatable, Sendable {
        package var data = Data()
        /// How many bytes were printed before `data`.
        package var start = 0
        package var generation = 0

        package init() {}

        package var end: Int { start + data.count }
    }

    package enum Action {
        /// It's shown: streams its output until it exits or is hidden.
        case attach
        case output(TerminalOutput)
        case detached(String?)
        case typed(Data)
        /// The keys sent before reached the host.
        case sent
        case typingFailed(String)
        case resized(TerminalSize)
        case failed(String)
    }

    private enum ResizeID: Hashable { case resize }

    @Dependency(\.daemon) var daemon
    @Dependency(\.continuousClock) var clock

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .attach:
                state.detached = false
                state.failure = nil
                let terminal = state.terminal
                return .run { send in
                    for try await output in await daemon.attachTerminal(terminal) {
                        await send(.output(output))
                    }
                } catch: { error, send in
                    // Closed elsewhere: there is nothing to attach to again.
                    if case DaemonError.refused(.unknownTerminal, let message) = error {
                        await send(.output(TerminalOutput(data: Data(), exitCode: -1)))
                        await send(.failed(message))
                    } else {
                        await send(.detached(error.localizedDescription))
                    }
                }

            case .output(let output):
                state.failure = nil
                state.retries = 0
                if output.replay {
                    // A reattach replays what's already drawn: it stays,
                    // with no redraw.
                    if output.data != state.screen.data {
                        state.screen.data = output.data
                        state.screen.start = 0
                        state.screen.generation += 1
                    }
                } else {
                    state.screen.data.append(output.data)
                }
                let over = state.screen.data.count - Self.kept
                if over > 0 {
                    state.screen.data.removeFirst(over)
                    state.screen.start += over
                }
                if let code = output.exitCode { state.terminal.exitCode = code }
                return .none

            case .detached(let reason):
                guard !state.exited else { return .none }
                state.detached = true
                state.failure = reason
                // Reattaching replays what was missed; a host that stays
                // away is asked less often, up to every 30 seconds.
                let wait = min(2 << min(state.retries, 4), 30)
                state.retries += 1
                return .run { send in
                    try await clock.sleep(for: .seconds(wait))
                    await send(.attach)
                }

            case .typed(let data):
                guard !state.exited else { return .none }
                state.pending.append(data)
                return send(&state)

            case .sent:
                state.sending = false
                return send(&state)

            case .typingFailed(let message):
                state.sending = false
                state.pending = Data()
                state.failure = message
                return .none

            case .resized(let size):
                guard !state.exited else { return .none }
                let terminal = state.terminal
                return .run { _ in
                    try await clock.sleep(for: .milliseconds(100))
                    try await daemon.resizeTerminal(terminal, size)
                } catch: { error, send in
                    guard !(error is CancellationError) else { return }
                    await send(.failed(error.localizedDescription))
                }
                .cancellable(id: ResizeID.resize, cancelInFlight: true)

            case .failed(let message):
                state.failure = message
                return .none
            }
        }
    }

    /// Sends the keys waiting, unless some are already on their way.
    private func send(_ state: inout State) -> Effect<Action> {
        guard !state.sending, !state.pending.isEmpty, !state.exited else { return .none }
        state.sending = true
        let (terminal, data) = (state.terminal, state.pending)
        state.pending = Data()
        return .run { send in
            try await daemon.typeInTerminal(terminal, data)
            await send(.sent)
        } catch: { error, send in
            await send(.typingFailed(error.localizedDescription))
        }
    }
}
