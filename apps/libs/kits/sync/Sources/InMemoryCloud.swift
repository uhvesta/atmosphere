import Foundation

/// A cloud in memory that several devices share, for tests and previews.
/// Like CloudKit, it refuses a record a device sends when another device
/// changed it since the sender last fetched it.
public final class InMemoryCloud<Snapshot: SyncSnapshot>: Sendable {
    private struct Stored {
        var change: CloudChange<Snapshot>
        var version: Int
    }

    private struct State {
        var items: [String: Stored] = [:]
        /// Each save in order, by name and version.
        var log: [(name: String, version: Int)] = []
        /// Per device, how far it has fetched in `log`.
        var fetched: [String: Int] = [:]
        /// Per device, the version of each item it last saw.
        var seen: [String: [String: Int]] = [:]
        var offline: Set<String> = []
    }

    private let state = Locked(State())

    public init() {}

    /// The cloud as `device` sees it.
    public func connection(device: String) -> Cloud<Snapshot> {
        Cloud(
            fetch: { [state] () throws -> [CloudChange<Snapshot>] in
                try state.withValue { state in
                    guard !state.offline.contains(device) else { throw SyncError.unreachable("offline") }
                    let start = state.fetched[device] ?? 0
                    var changes: [String: CloudChange<Snapshot>] = [:]
                    var order: [String] = []
                    for (name, version) in state.log[start...] {
                        guard let stored = state.items[name], stored.version == version else { continue }
                        state.seen[device, default: [:]][name] = version
                        guard stored.change.device != device else { continue }
                        if changes[name] == nil { order.append(name) }
                        changes[name] = stored.change
                    }
                    state.fetched[device] = state.log.count
                    return order.compactMap { changes[$0] }
                }
            },
            send: { [state] items throws -> [CloudChange<Snapshot>] in
                try state.withValue { state in
                    guard !state.offline.contains(device) else { throw SyncError.unreachable("offline") }
                    var refused: [CloudChange<Snapshot>] = []
                    for item in items {
                        let name = item.name
                        let current = state.items[name]
                        if let current, current.change.device != device,
                            current.version > state.seen[device]?[name] ?? 0
                        {
                            // As CloudKit hands back the server's record.
                            state.seen[device, default: [:]][name] = current.version
                            refused.append(current.change)
                            continue
                        }
                        let version = (current?.version ?? 0) + 1
                        state.items[name] = Stored(change: CloudChange(item, from: device), version: version)
                        state.log.append((name, version))
                        state.seen[device, default: [:]][name] = version
                    }
                    return refused
                }
            })
    }

    /// Whether `device` can reach the cloud.
    public func setOnline(_ online: Bool, device: String) {
        state.withValue { state in
            if online { state.offline.remove(device) } else { state.offline.insert(device) }
        }
    }

    /// Every item the cloud has, by name, for tests.
    public var items: [String: CloudItem<Snapshot>] {
        state.value.items.mapValues(\.change.item)
    }
}
