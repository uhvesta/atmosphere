import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// Terminals at one place, a session's folder or a host, as tabs: those the
/// host already runs there, opening another, and closing one. Each keeps
/// running on its host while nothing shows it.
@Reducer
package struct TerminalsFeature {
    /// Identified by its place, so another place's terminals shown in its
    /// stead stop what this one was doing instead of finishing it into them.
    @ObservableState
    package struct State: Equatable, Identifiable {
        package var id: [String?] { [place.host, place.sessionID, place.folder] }
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

        /// Why the terminals couldn't be read, while there's none to show
        /// in their place.
        package var listFailure: String? { listed || opening || !tabs.isEmpty ? nil : failure }

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
                // Asked again after a list that failed, it shows it's reading.
                if !state.listed { state.failure = nil }
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
                guard let tab = state.tabs[id: id] else { return .none }
                // Closed elsewhere already: only the tab is left to drop.
                guard !tab.gone else { return .send(.closed(id)) }
                let terminal = tab.terminal
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

/// One terminal, drawn by a surface that runs a command piping it from its
/// host. When that command ends, the host says whether the shell exited,
/// the terminal was closed elsewhere, or only the pipe ended, which
/// Reattach starts again.
@Reducer
package struct TerminalFeature {
    @ObservableState
    package struct State: Equatable, Identifiable {
        package var terminal: Terminal
        /// What the surface runs, once the terminal is shown.
        package var command: TerminalCommand?
        /// Which surface draws it: Reattach makes the next.
        package var generation = 0
        /// Whether the surface's command ended while the shell still runs.
        package var detached = false
        /// Whether the host no longer has it: closed from elsewhere.
        package var gone = false
        /// Why the host couldn't be asked about it, and what to do about it.
        package var failure: String?

        package init(terminal: Terminal) {
            self.terminal = terminal
        }

        package var id: Terminal.ID { terminal.id }
        /// Whether its shell is over, by exiting or being closed.
        package var exited: Bool { terminal.exitCode != nil || gone }
    }

    package enum Action {
        /// It's shown: works out the command its surface runs.
        case attach
        /// The surface's command ended: asks the host what became of it.
        case surfaceExited
        /// What the host says of it now; nil when it has none such.
        case checked(Terminal?)
        case reattachTapped
        case failed(String)
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.terminalCommand) var terminalCommand

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .attach:
                if state.command == nil { state.command = terminalCommand.command(state.terminal) }
                return .none

            case .surfaceExited:
                guard !state.exited else { return .none }
                let terminal = state.terminal
                return .run { send in
                    let terminals = try await daemon.terminals(terminal.host)
                    await send(.checked(terminals.first { $0.id == terminal.id }))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .checked(let terminal):
                state.failure = nil
                guard let terminal else {
                    state.gone = true
                    state.detached = false
                    return .none
                }
                state.terminal = terminal
                state.detached = terminal.exitCode == nil
                return .none

            case .reattachTapped:
                guard !state.exited else { return .none }
                state.detached = false
                state.failure = nil
                state.generation += 1
                return .none

            case .failed(let message):
                // The host couldn't say: Reattach tries the pipe again.
                state.detached = true
                state.failure = message
                return .none
            }
        }
    }
}
