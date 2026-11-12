import Foundation
import JournalKit

/// Where sync is, for settings and the review banner.
public struct SyncState<Record: Journaled>: Hashable, Sendable {
    public enum Status: Hashable, Sendable {
        case idle
        case syncing
        /// The last sync didn't finish; it's tried again on the next one.
        case failed(SyncError)
    }

    public var status: Status
    public var lastSynced: Date?
    /// Records that couldn't merge, for the person to review.
    public var review: [ReviewItem<Record>]

    public init(status: Status = .idle, lastSynced: Date? = nil, review: [ReviewItem<Record>] = []) {
        self.status = status
        self.lastSynced = lastSynced
        self.review = review
    }
}

/// Where the ledger is kept between launches.
public struct LedgerStorage<Snapshot: SyncSnapshot>: Sendable {
    /// Throws `SyncError.ledgerFailed`.
    public var load: @Sendable () throws -> SyncLedger<Snapshot>
    /// Throws `SyncError.ledgerFailed`.
    public var save: @Sendable (SyncLedger<Snapshot>) throws -> Void

    public init(
        load: @escaping @Sendable () throws -> SyncLedger<Snapshot>,
        save: @escaping @Sendable (SyncLedger<Snapshot>) throws -> Void
    ) {
        self.load = load
        self.save = save
    }

    /// Kept in memory, for tests and previews.
    public static func inMemory(_ ledger: SyncLedger<Snapshot> = SyncLedger()) -> Self {
        let stored = Locked(ledger)
        return Self(load: { stored.value }, save: { ledger in stored.withValue { $0 = ledger } })
    }
}

/// The app's records, as sync reads and writes them.
public struct SyncStore<Snapshot: SyncSnapshot>: Sendable {
    public typealias RecordKey = RecordID<Snapshot.Record.Kind>

    /// The records now.
    public var load: @Sendable () async throws -> Snapshot
    /// Writes what other devices changed, and returns the records after.
    public var importSynced: @Sendable (Synced<Snapshot>) async throws -> Snapshot
    /// Writes the person's choices for records under review, as one change
    /// they can undo.
    public var resolve: @Sendable ([RecordKey: Snapshot.Record?]) async throws -> Void

    public init(
        load: @escaping @Sendable () async throws -> Snapshot,
        importSynced: @escaping @Sendable (Synced<Snapshot>) async throws -> Snapshot,
        resolve: @escaping @Sendable ([RecordKey: Snapshot.Record?]) async throws -> Void
    ) {
        self.load = load
        self.importSynced = importSynced
        self.resolve = resolve
    }
}

/// Syncs one device's records with the cloud: fetches what other devices
/// sent, merges it, holds what can't merge for review, and sends the rest.
/// One sync runs at a time; asking during one waits for it.
public actor Syncer<Snapshot: SyncSnapshot> {
    public typealias State = SyncState<Snapshot.Record>

    private let cloud: Cloud<Snapshot>
    private let store: SyncStore<Snapshot>
    private let storage: LedgerStorage<Snapshot>
    private let now: @Sendable () -> Date
    private var ledger: SyncLedger<Snapshot>?
    private var state = State()
    private var running: Task<State, Never>?
    /// A sync was asked for while one ran: that run loaded the records
    /// before, so it runs again to send what changed since.
    private var runsAgain = false
    private let states = Locked<[UUID: AsyncStream<State>.Continuation]>([:])

    public init(
        cloud: Cloud<Snapshot>, store: SyncStore<Snapshot>, storage: LedgerStorage<Snapshot>,
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.cloud = cloud
        self.store = store
        self.storage = storage
        self.now = now
    }

    /// The state now, and after each change.
    public func stream() -> AsyncStream<State> {
        let (stream, continuation) = AsyncStream<State>.makeStream()
        let id = UUID()
        states.withValue { $0[id] = continuation }
        continuation.onTermination = { [states] _ in states.withValue { $0[id] = nil } }
        continuation.yield(state)
        return stream
    }

    public func current() -> State { state }

    /// Fetches, merges and sends until the cloud and this device agree on
    /// every record that isn't held.
    @discardableResult
    public func sync() async -> State {
        if let running {
            runsAgain = true
            return await running.value
        }
        let task = Task {
            var result = await run()
            while runsAgain {
                runsAgain = false
                result = await run()
            }
            return result
        }
        running = task
        let result = await task.value
        running = nil
        return result
    }

    /// Writes the person's choices for the held records, then syncs to send
    /// them.
    @discardableResult
    public func resolve(_ choices: ReviewChoices<Snapshot.Record>) async -> State {
        if let running { _ = await running.value }
        do {
            var ledger = try loadLedger()
            let snapshot = try await load()
            let resolved = ledger.resolve(choices, in: snapshot)
            if !resolved.isEmpty {
                do {
                    try await store.resolve(resolved)
                } catch {
                    throw SyncError.storeFailed(String(describing: error))
                }
            }
            try keep(ledger)
        } catch {
            publish(status: .failed(Self.syncError(error)), review: state.review)
            return state
        }
        return await sync()
    }

    private func run() async -> State {
        publish(status: .syncing, review: state.review)
        do {
            var ledger = try loadLedger()
            var snapshot = try await load()
            let fetched = try await cloud.fetch()
            snapshot = try await receive(fetched, into: snapshot, &ledger)
            // Sending again after the cloud refuses an item, with the other
            // device's version merged in. A few rounds settle a busy record.
            for _ in 0..<4 {
                let items = ledger.pending(snapshot)
                guard !items.isEmpty else { break }
                let refused = try await cloud.send(items)
                let names = Set(refused.map(\.item.name))
                ledger.saved(items.filter { !names.contains($0.name) })
                try keep(ledger)
                guard !refused.isEmpty else { break }
                snapshot = try await receive(refused, into: snapshot, &ledger)
            }
            ledger.lastSynced = now()
            try keep(ledger)
            state.lastSynced = ledger.lastSynced
            publish(status: .idle, review: ledger.conflicts(snapshot))
        } catch {
            publish(status: .failed(Self.syncError(error)), review: state.review)
        }
        return state
    }

    private func receive(
        _ changes: [CloudChange<Snapshot>], into snapshot: Snapshot, _ ledger: inout SyncLedger<Snapshot>
    ) async throws -> Snapshot {
        guard !changes.isEmpty else { return snapshot }
        // The ledger only learns what the cloud has once the store has it
        // too: a ledger kept ahead of a failed import would send this
        // device's older values back over the other device's change.
        var received = ledger
        let synced = received.receive(changes, into: snapshot)
        var imported = snapshot
        if !synced.isEmpty {
            do {
                imported = try await store.importSynced(synced)
            } catch {
                throw SyncError.storeFailed(String(describing: error))
            }
        }
        ledger = received
        try keep(ledger)
        return imported
    }

    private func load() async throws -> Snapshot {
        do {
            return try await store.load()
        } catch {
            throw SyncError.storeFailed(String(describing: error))
        }
    }

    private func loadLedger() throws -> SyncLedger<Snapshot> {
        if let ledger { return ledger }
        let loaded = try storage.load()
        ledger = loaded
        state.lastSynced = loaded.lastSynced
        return loaded
    }

    private func keep(_ ledger: SyncLedger<Snapshot>) throws {
        self.ledger = ledger
        try storage.save(ledger)
    }

    private func publish(status: State.Status, review: [ReviewItem<Snapshot.Record>]) {
        state.status = status
        state.review = review
        for continuation in states.value.values { continuation.yield(state) }
    }

    private static func syncError(_ error: any Error) -> SyncError {
        (error as? SyncError) ?? .refused(String(describing: error))
    }
}
