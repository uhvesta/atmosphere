import ComposableArchitecture
import Foundation
import LifrCore

extension StoreClient {
    /// Keeps the records in memory, starting from `workspace`. Changes use
    /// the `date`, `uuid` and `withRandomNumberGenerator` dependencies, so
    /// tests can fix them.
    package static func inMemory(_ workspace: Workspace, actor: Actor = .user(device: "This device", surface: .iPhone))
        -> Self
    {
        inMemorySyncing(workspace, actor: actor).client
    }

    /// `inMemory`, with a way to change the records as a sync import would:
    /// `sync` applies records from another device, reconciles them, and
    /// sends the result to `updates`.
    package static func inMemorySyncing(
        _ workspace: Workspace, actor: Actor = .user(device: "This device", surface: .iPhone)
    ) -> (client: Self, sync: InMemorySync) {
        let records = Records(workspace)
        let sync = InMemorySync(records: records, actor: actor)
        let archived = LockIsolated<[ArchivedEntry]>([])

        let client = StoreClient(
            load: { records.value },
            snapshot: { records.snapshot },
            save: { operation in try records.save(operation, as: actor) },
            updates: { sync.stream() },
            importSynced: { changes in
                @Dependency(\.date.now) var now
                return sync(now: now) { workspace in
                    workspace.importSynced(
                        records: changes.records, journal: changes.journal, history: changes.history,
                        seen: changes.seen)
                }
            },
            archiveJournal: { cutoff in
                let packed = records.update { $0.archiveJournal(before: cutoff) }
                archived.withValue { $0 = $0.merging(packed) }
            },
            archivedJournal: { archived.value }
        )
        return (client, sync)
    }
}

/// Imports records into an in-memory store, as CloudKit would.
package final class InMemorySync: Sendable {
    private let records: Records
    private let actor: Actor
    private let continuations = LockIsolated<[AsyncStream<Update>.Continuation]>([])

    fileprivate init(records: Records, actor: Actor) {
        self.records = records
        self.actor = actor
    }

    /// Another writer on the same records, such as Shortcuts, making its
    /// changes as `actor`. The first client's `updates` sends on each change
    /// it saves, as the live store's does for another process. A writer
    /// hears no updates and imports nothing from sync.
    package func writer(as actor: Actor) -> StoreClient {
        var writer = StoreClient()
        writer.load = { [records] in records.value }
        writer.snapshot = { [records] in records.snapshot }
        writer.save = { [records, continuations] operation in
            let saved = try records.save(operation, as: actor)
            let update = Update(workspace: saved.workspace, fixes: [], number: saved.number)
            for continuation in continuations.value { continuation.yield(update) }
            return saved
        }
        return writer
    }

    fileprivate func stream() -> AsyncStream<Update> {
        let (stream, continuation) = AsyncStream<Update>.makeStream()
        continuations.withValue { $0.append(continuation) }
        return stream
    }

    /// Runs `body` on the records as another device's changes arriving,
    /// then reconciles them at `now` and sends the result to `updates`.
    @discardableResult
    package func callAsFunction(now: Date, _ body: (inout Workspace) -> Void) -> Update {
        let update = records.numbered { records, number in
            body(&records)
            var fixes: [Fix] = []
            let context = Context(actor: .system(device: actor.device, surface: actor.surface), now: now)
            records.perform(in: context) { records, context in
                fixes = records.reconcile(in: context)
                return Change()
            }
            return Update(workspace: records, fixes: fixes, number: number)
        }
        for continuation in continuations.value { continuation.yield(update) }
        return update
    }
}

/// The in-memory records. Each update runs alone.
private final class Records: @unchecked Sendable {
    private let lock = NSLock()
    private var workspace: Workspace
    /// How many saves and updates there have been, to number them.
    private var changes = 0

    init(_ workspace: Workspace) {
        self.workspace = workspace
    }

    var value: Workspace { lock.withLock { workspace } }

    var snapshot: Snapshot { lock.withLock { Snapshot(workspace, number: changes) } }

    /// Performs `operation` as `actor`, with the `date`, `uuid` and
    /// `withRandomNumberGenerator` dependencies.
    func save(_ operation: StoreOperation, as actor: Actor) throws(LifrError) -> Saved {
        @Dependency(\.date.now) var now
        @Dependency(\.uuid) var uuid
        @Dependency(\.withRandomNumberGenerator) var random
        let context = Context(
            actor: actor, now: now, newID: { uuid() },
            newCode: { random { TaskCode.random(using: &$0) } })
        return try numbered { (records, number) throws(LifrError) -> Saved in
            var copy = records
            let change = try copy.perform(in: context, operation)
            records = copy
            return Saved(workspace: copy, change: change, number: number)
        }
    }

    /// Runs `body` alone with the next number among the saves and updates,
    /// which it takes only if `body` returns.
    func numbered<T, E: Error>(_ body: (inout Workspace, Int) throws(E) -> T) throws(E) -> T {
        lock.lock()
        defer { lock.unlock() }
        let result = try body(&workspace, changes + 1)
        changes += 1
        return result
    }

    func update<T, E: Error>(_ body: (inout Workspace) throws(E) -> T) throws(E) -> T {
        lock.lock()
        defer { lock.unlock() }
        return try body(&workspace)
    }
}
