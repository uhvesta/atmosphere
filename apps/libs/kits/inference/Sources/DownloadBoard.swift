import Foundation

/// The downloads in progress, for everyone watching them.
public final class DownloadBoard: Sendable {
    private struct State {
        var downloads: [ModelID: DownloadState] = [:]
        var watchers: [UUID: AsyncStream<[ModelID: DownloadState]>.Continuation] = [:]
    }

    private let state = Locked(State())

    public init() {}

    public var downloads: [ModelID: DownloadState] { state.value.downloads }

    /// Changes the downloads and tells everyone watching.
    public func update(_ change: (inout [ModelID: DownloadState]) -> Void) {
        let (downloads, watchers) = state.withValue { state in
            change(&state.downloads)
            return (state.downloads, Array(state.watchers.values))
        }
        for watcher in watchers { watcher.yield(downloads) }
    }

    /// The downloads now, then each time they change.
    public func stream() -> AsyncStream<[ModelID: DownloadState]> {
        AsyncStream { continuation in
            let id = UUID()
            let current = state.withValue { state in
                state.watchers[id] = continuation
                return state.downloads
            }
            continuation.yield(current)
            continuation.onTermination = { [state] _ in
                state.withValue { _ = $0.watchers.removeValue(forKey: id) }
            }
        }
    }
}
