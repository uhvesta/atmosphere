import Foundation
import Humata2Core
import Observation

/// The session list as the core last told it, on the main actor.
@MainActor
@Observable
public final class SessionsModel {
    public private(set) var list = SessionList(rows: [], loading: true, error: nil, selected: nil)
    /// Why the core couldn't start; nothing else works then.
    public private(set) var startError: String?
    /// The chat of the session picked.
    public let chat: ChatModel

    @ObservationIgnored private let core: Core?
    @ObservationIgnored private var watch: Watch?

    /// Reaches the daemon at `socket`, or at its usual place.
    public init(socket: String? = ProcessInfo.processInfo.environment["HUMATA_SOCKET"]) {
        let core: Core?
        do {
            core = try Core(socket: socket)
        } catch {
            core = nil
            startError = "\(error)"
        }
        self.core = core
        chat = ChatModel(core: core)
        watch = core?.observe(
            observer: Relay { [weak self] list in
                DispatchQueue.main.async {
                    MainActor.assumeIsolated { self?.list = list }
                }
            })
    }

    public func send(_ event: SessionsEvent) {
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
