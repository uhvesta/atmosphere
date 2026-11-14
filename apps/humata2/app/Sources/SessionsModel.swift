import Foundation
import Humata2Core
import Observation

/// The session list as the core last told it, on the main actor.
@MainActor
@Observable
final class SessionsModel {
    private(set) var list = SessionList(rows: [], loading: true, error: nil, selected: nil)
    /// Why the core couldn't start; nothing else works then.
    private(set) var startError: String?

    @ObservationIgnored private let core: Core?
    @ObservationIgnored private var watch: Watch?

    init() {
        let socket = ProcessInfo.processInfo.environment["HUMATA_SOCKET"]
        let core: Core?
        do {
            core = try Core(socket: socket)
        } catch {
            core = nil
            startError = "\(error)"
        }
        self.core = core
        watch = core?.observe(
            observer: Relay { [weak self] list in
                Task { @MainActor in self?.list = list }
            })
    }

    func send(_ event: SessionsEvent) {
        core?.send(event: event)
    }
}

/// Passes what the core tells, from whichever thread it tells it.
private final class Relay: SessionsObserver {
    private let changed: @Sendable (SessionList) -> Void

    init(_ changed: @escaping @Sendable (SessionList) -> Void) {
        self.changed = changed
    }

    func changed(list: SessionList) {
        changed(list)
    }
}
