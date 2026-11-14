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
        /// The tabs whose shell the host was told to end and hasn't
        /// answered for: each shows it, and takes no second go.
        package var closing: Set<Terminal.ID> = []
        /// Asks before ending a shell that hasn't exited, since what runs
        /// in it stops and can't be brought back.
        @Presents package var confirm: AlertState<Action.Confirm>?

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
        /// The tabs appeared: reads the terminals running here. The read,
        /// and the first open it may lead to, go on when the tabs are left,
        /// so the shell that opens is the one shown on return.
        case task
        case listed([Terminal])
        case newTapped
        case opened(Terminal)
        case selected(Terminal.ID)
        case closeTapped(Terminal.ID)
        case closed(Terminal.ID)
        /// The host couldn't end the tab's shell, and this is why.
        case closeFailed(Terminal.ID, String)
        case failed(String)
        case confirm(PresentationAction<Confirm>)
        case terminals(IdentifiedActionOf<TerminalFeature>)

        package enum Confirm: Equatable, Sendable {
            /// End the tab's shell, though it hasn't exited.
            case endShell(Terminal.ID)
        }
    }

    @Dependency(\.daemon) var daemon
    @Dependency(FirstOpens.self) var firstOpens

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                // Asked again after a list that failed, it shows it's reading.
                if !state.listed { state.failure = nil }
                let host = state.place.host
                // A first shell asked for here before these tabs were made,
                // by a session since left, is waited for: the host lists it
                // once it has opened, and no second is asked for.
                let awaited = state.opensFirst && !state.opening ? state.id : nil
                return .run { send in
                    if let awaited { await firstOpens.settled(awaited) }
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
                guard first, state.tabs.isEmpty, state.opensFirst, !state.opening else { return .none }
                return .send(.newTapped)

            case .newTapped:
                guard !state.opening else { return .none }
                state.opening = true
                let place = state.place
                // A session's first shell goes on opening when the session
                // is left, and is the one any later ask for it gets.
                guard state.opensFirst, state.tabs.isEmpty else {
                    return .run { send in
                        await send(.opened(try await daemon.openTerminal(place, .standard)))
                    } catch: { error, send in
                        await send(.failed(error.localizedDescription))
                    }
                }
                let key = state.id
                return .run { send in
                    let terminal = try await firstOpens.open(key) { try await daemon.openTerminal(place, .standard) }
                    await send(.opened(terminal))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .opened(let terminal):
                state.opening = false
                state.failure = nil
                if state.tabs[id: terminal.id] == nil {
                    state.tabs.append(TerminalFeature.State(terminal: terminal))
                }
                state.selected = terminal.id
                return .none

            case .selected(let id):
                state.selected = id
                return .none

            case .closeTapped(let id):
                guard let tab = state.tabs[id: id], !state.closing.contains(id) else { return .none }
                // Closed elsewhere already: only the tab is left to drop.
                guard !tab.gone else { return .send(.closed(id)) }
                // Its shell exited: there's nothing running to lose.
                guard !tab.exited else { return close(id, &state) }
                // An idle prompt can't be told from a running job, and
                // these shells outlive the app, so it always asks.
                let title = tab.terminal.title
                state.confirm = AlertState {
                    TextState(title.isEmpty ? "End this terminal’s shell?" : "End the shell of “\(title)”?")
                } actions: {
                    ButtonState(role: .destructive, action: .endShell(id)) { TextState("End Shell") }
                    ButtonState(role: .cancel) { TextState("Cancel") }
                } message: {
                    TextState(
                        "Whatever is running in it stops, and what it printed is gone. To keep it running, "
                            + "cancel: the shell stays on its host while no tab shows it.")
                }
                return .none

            case .confirm(.presented(.endShell(let id))):
                return close(id, &state)

            case .confirm:
                return .none

            case .closeFailed(let id, let message):
                state.closing.remove(id)
                state.failure = message
                return .none

            case .closed(let id):
                state.closing.remove(id)
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
        .ifLet(\.$confirm, action: \.confirm)
        .forEach(\.tabs, action: \.terminals) {
            TerminalFeature()
        }
    }

    /// Tells the host to end the tab's shell, once: the tab shows it's
    /// closing until the host answers.
    private func close(_ id: Terminal.ID, _ state: inout State) -> Effect<Action> {
        guard let terminal = state.tabs[id: id]?.terminal, state.closing.insert(id).inserted else { return .none }
        return .run { send in
            try await daemon.closeTerminal(terminal)
            await send(.closed(id))
        } catch: { error, send in
            await send(.closeFailed(id, error.localizedDescription))
        }
    }
}

/// The first shells being opened, by place. One goes on opening when the
/// tabs that asked for it are gone, and whoever asks for the same place's
/// first shell meanwhile gets that one.
package final class FirstOpens: Sendable {
    private let opening = LockIsolated<[[String?]: Task<Terminal, any Error>]>([:])

    package init() {}

    /// The shell `open` makes at `place`, or the one already being made
    /// there. Ending the caller's task doesn't end the open.
    func open(_ place: [String?], _ open: @escaping @Sendable () async throws -> Terminal) async throws -> Terminal {
        let task = opening.withValue { tasks -> Task<Terminal, any Error> in
            if let task = tasks[place] { return task }
            let task = Task<Terminal, any Error> { [opening] in
                defer { opening.withValue { $0[place] = nil } }
                return try await open()
            }
            tasks[place] = task
            return task
        }
        return try await task.value
    }

    /// Returns once no first shell is being opened at `place`, whether the
    /// one that was opened or failed.
    func settled(_ place: [String?]) async {
        guard let task = opening.withValue({ $0[place] }) else { return }
        _ = try? await task.value
    }
}

extension FirstOpens: DependencyKey {
    package static let liveValue = FirstOpens()
    /// Each test's own, so one test's slow open isn't joined by another's.
    package static var testValue: FirstOpens { FirstOpens() }
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
