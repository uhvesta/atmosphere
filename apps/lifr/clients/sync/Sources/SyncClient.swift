import ComposableArchitecture
import Foundation
import LifrStoreClient

/// Syncs Lifr's records with the person's other devices.
@DependencyClient
package struct SyncClient: Sendable {
    /// The state now, then after each change. Ends when the calling task is
    /// cancelled.
    package var states: @Sendable () async -> AsyncStream<SyncState> = { AsyncStream { $0.finish() } }
    /// Syncs now, and returns how it went. Failures are in the state's
    /// status, and the next sync tries again.
    package var sync: @Sendable () async -> SyncState = { SyncState() }
    /// Writes the person's choices for the records under review, then syncs.
    package var resolve: @Sendable (_ choices: Review.Choices) async -> SyncState = { _ in SyncState() }
    /// This device changed the records: sync soon, once the changes settle.
    package var changed: @Sendable () async -> Void
}

extension SyncClient {
    /// Syncs through `syncer`. `changed` waits `settle` for more changes
    /// before syncing.
    package static func syncing(_ syncer: Syncer, settle: Duration = .seconds(5)) -> Self {
        let soon = LockIsolated<Task<Void, Never>?>(nil)
        return SyncClient(
            states: { await syncer.stream() },
            sync: { await syncer.sync() },
            resolve: { choices in await syncer.resolve(choices) },
            changed: {
                soon.withValue { task in
                    task?.cancel()
                    task = Task {
                        try? await Task.sleep(for: settle)
                        guard !Task.isCancelled else { return }
                        await syncer.sync()
                    }
                }
            })
    }

    /// Never syncs, such as in previews.
    package static let off = SyncClient(
        states: { AsyncStream { $0.yield(SyncState()) } }, sync: { SyncState() }, resolve: { _ in SyncState() },
        changed: {})
}

extension StoreClient {
    /// This store, telling `sync` after each change it saves.
    package func telling(_ sync: SyncClient) -> Self {
        var store = self
        let save = self.save
        store.save = { operation in
            let saved = try await save(operation)
            if saved.change.entry != nil { await sync.changed() }
            return saved
        }
        return store
    }
}

extension SyncClient: TestDependencyKey {
    package static let testValue = SyncClient()
    package static let previewValue = SyncClient.off
}

extension DependencyValues {
    package var sync: SyncClient {
        get { self[SyncClient.self] }
        set { self[SyncClient.self] = newValue }
    }
}
