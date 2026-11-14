import Foundation
import Humata2Core
import Observation

/// The open chat, kept from the core's updates on the main actor. The rows
/// and the streaming reply are watched apart, so a reply growing redraws
/// only itself.
@MainActor
@Observable
public final class ChatModel {
    public private(set) var rows: [Row] = []
    public private(set) var reply: StreamingReply?
    public private(set) var status = ChatStatus(
        session: nil, reading: nil, working: false, waiting: nil, queued: [], sending: false,
        stopping: false, unsent: nil, error: nil)

    @ObservationIgnored private let core: Core?
    @ObservationIgnored private var watch: Watch?

    init(core: Core?) {
        self.core = core
        watch = core?.observeChat(
            observer: ChatRelay { [weak self] update in
                // In order: each update builds on the one before.
                DispatchQueue.main.async {
                    MainActor.assumeIsolated { self?.apply(update) }
                }
            })
    }

    public func send(_ event: ChatEvent) {
        core?.sendChat(event: event)
    }

    private func apply(_ update: ChatUpdate) {
        if update.reset {
            rows.removeAll()
            reply = nil
        }
        if !update.rows.isEmpty {
            var changed = rows
            for change in update.rows {
                switch change {
                case .removed(let id):
                    changed.removeAll { $0.id == id }
                case .inserted(let index, let row):
                    changed.insert(row, at: Int(index))
                case .updated(let index, let row):
                    changed[Int(index)] = row
                }
            }
            rows = changed
        }
        switch update.reply {
        case .started(let id, let text): reply = StreamingReply(id: id, text: text)
        case .appended(let text): reply?.text += text
        case .ended: reply = nil
        case nil: break
        }
        if let status = update.status {
            self.status = status
        }
    }
}

/// The reply as it streams.
public struct StreamingReply: Equatable, Sendable {
    public let id: String
    public var text: String
}

/// Passes what the core tells, from whichever thread it tells it.
private final class ChatRelay: ChatObserver {
    private let changed: @Sendable (ChatUpdate) -> Void

    init(_ changed: @escaping @Sendable (ChatUpdate) -> Void) {
        self.changed = changed
    }

    func changed(update: ChatUpdate) {
        changed(update)
    }
}
