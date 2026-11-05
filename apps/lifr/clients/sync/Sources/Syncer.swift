import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient

/// Where sync is, for Settings and the review banner.
package struct SyncState: Hashable, Sendable {
    package enum Status: Hashable, Sendable {
        case idle
        case syncing
        /// The last sync didn't finish; it's tried again on the next one.
        case failed(SyncError)
    }

    package var status: Status
    package var lastSynced: Date?
    /// Records that couldn't merge, for the person to review.
    package var review: [Review.Item]

    package init(status: Status = .idle, lastSynced: Date? = nil, review: [Review.Item] = []) {
        self.status = status
        self.lastSynced = lastSynced
        self.review = review
    }
}

/// Where the ledger is kept between launches.
package struct LedgerStorage: Sendable {
    /// Throws `SyncError.ledgerFailed`.
    package var load: @Sendable () throws -> SyncLedger
    /// Throws `SyncError.ledgerFailed`.
    package var save: @Sendable (SyncLedger) throws -> Void

    package init(
        load: @escaping @Sendable () throws -> SyncLedger, save: @escaping @Sendable (SyncLedger) throws -> Void
    ) {
        self.load = load
        self.save = save
    }

    /// Kept in memory, for tests and previews.
    package static func inMemory(_ ledger: SyncLedger = SyncLedger()) -> Self {
        let stored = LockIsolated(ledger)
        return Self(load: { stored.value }, save: { ledger in stored.setValue(ledger) })
    }
}

/// Syncs one device's records with the cloud: fetches what other devices
/// sent, merges it, holds what can't merge for review, and sends the rest.
/// One sync runs at a time; asking during one waits for it.
package actor Syncer {
    private let cloud: Cloud
    private let store: StoreClient
    private let storage: LedgerStorage
    private let now: @Sendable () -> Date
    private var ledger: SyncLedger?
    private var state = SyncState()
    private var running: Task<SyncState, Never>?
    private let states = LockIsolated<[UUID: AsyncStream<SyncState>.Continuation]>([:])

    package init(
        cloud: Cloud, store: StoreClient, storage: LedgerStorage, now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.cloud = cloud
        self.store = store
        self.storage = storage
        self.now = now
    }

    /// The state now, and after each change.
    package func stream() -> AsyncStream<SyncState> {
        let (stream, continuation) = AsyncStream<SyncState>.makeStream()
        let id = UUID()
        states.withValue { $0[id] = continuation }
        continuation.onTermination = { [states] _ in states.withValue { $0[id] = nil } }
        continuation.yield(state)
        return stream
    }

    package func current() -> SyncState { state }

    /// Fetches, merges and sends until the cloud and this device agree on
    /// every record that isn't held.
    @discardableResult
    package func sync() async -> SyncState {
        if let running { return await running.value }
        let task = Task { await run() }
        running = task
        let result = await task.value
        running = nil
        return result
    }

    /// Writes the person's choices for the held records as one journal
    /// entry, so they can be undone, then syncs to send them.
    @discardableResult
    package func resolve(_ choices: Review.Choices) async -> SyncState {
        if let running { _ = await running.value }
        do {
            var ledger = try loadLedger()
            let workspace = try await load()
            let resolved = ledger.resolve(choices, in: workspace)
            if !resolved.isEmpty {
                do {
                    _ = try await store.save { workspace, _ in workspace.resolveSynced(resolved) }
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

    private func run() async -> SyncState {
        publish(status: .syncing, review: state.review)
        do {
            var ledger = try loadLedger()
            var workspace = try await load()
            let fetched = try await cloud.fetch()
            workspace = try await receive(fetched, into: workspace, &ledger)
            // Sending again after the cloud refuses an item, with the other
            // device's version merged in. A few rounds settle a busy record.
            for _ in 0..<4 {
                let items = ledger.pending(workspace)
                guard !items.isEmpty else { break }
                let refused = try await cloud.send(items)
                let names = Set(refused.map(\.item.name))
                ledger.saved(items.filter { !names.contains($0.name) })
                try keep(ledger)
                guard !refused.isEmpty else { break }
                workspace = try await receive(refused, into: workspace, &ledger)
            }
            ledger.lastSynced = now()
            try keep(ledger)
            state.lastSynced = ledger.lastSynced
            publish(status: .idle, review: ledger.conflicts(workspace))
        } catch {
            publish(status: .failed(Self.syncError(error)), review: state.review)
        }
        return state
    }

    private func receive(
        _ changes: [CloudChange], into workspace: Workspace, _ ledger: inout SyncLedger
    ) async throws -> Workspace {
        guard !changes.isEmpty else { return workspace }
        let synced = ledger.receive(changes, into: workspace)
        try keep(ledger)
        guard !synced.isEmpty else { return workspace }
        do {
            return try await store.importSynced(synced).workspace
        } catch {
            throw SyncError.storeFailed(String(describing: error))
        }
    }

    private func load() async throws -> Workspace {
        do {
            return try await store.load()
        } catch {
            throw SyncError.storeFailed(String(describing: error))
        }
    }

    private func loadLedger() throws -> SyncLedger {
        if let ledger { return ledger }
        let loaded = try storage.load()
        ledger = loaded
        state.lastSynced = loaded.lastSynced
        return loaded
    }

    private func keep(_ ledger: SyncLedger) throws {
        self.ledger = ledger
        try storage.save(ledger)
    }

    private func publish(status: SyncState.Status, review: [Review.Item]) {
        state.status = status
        state.review = review
        for continuation in states.value.values { continuation.yield(state) }
    }

    private static func syncError(_ error: any Error) -> SyncError {
        (error as? SyncError) ?? .refused(String(describing: error))
    }
}
