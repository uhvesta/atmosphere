import CloudKit
import Foundation
import Network

#if os(iOS)
    import UIKit
#endif

/// Each time the network comes back after being away, and each time the
/// iCloud account changes, such as signing in. A path that was fine from
/// the start isn't a return.
@Sendable func networkReturns() -> AsyncStream<Void> {
    AsyncStream { continuation in
        nonisolated(unsafe) let account = NotificationCenter.default.addObserver(
            forName: .CKAccountChanged, object: nil, queue: nil
        ) { _ in continuation.yield() }
        let monitor = NWPathMonitor()
        let wasAway = LockedFlag()
        monitor.pathUpdateHandler = { path in
            if path.status == .satisfied {
                if wasAway.take() { continuation.yield() }
            } else {
                wasAway.set()
            }
        }
        continuation.onTermination = { _ in
            monitor.cancel()
            NotificationCenter.default.removeObserver(account)
        }
        monitor.start(queue: DispatchQueue(label: "lifr.network", qos: .utility))
    }
}

/// Runs `work`, asking iOS for the few seconds it needs to finish when Lifr
/// is on its way to the background.
func finishingInBackground<T: Sendable>(_ work: @Sendable () async -> T) async -> T {
    #if os(iOS)
        let task = await MainActor.run { UIApplication.shared.beginBackgroundTask(withName: "Sync") }
        let result = await work()
        await MainActor.run { UIApplication.shared.endBackgroundTask(task) }
        return result
    #else
        return await work()
    #endif
}

/// Runs `work` without App Nap stretching it out, so a Mac with no window
/// open still syncs on time.
func awake<T: Sendable>(_ work: @Sendable () async -> T) async -> T {
    let activity = ProcessInfo.processInfo.beginActivity(
        options: [.userInitiatedAllowingIdleSystemSleep], reason: "Syncing with other devices")
    defer { ProcessInfo.processInfo.endActivity(activity) }
    return await work()
}

private final class LockedFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var isSet = false

    func set() { lock.withLock { isSet = true } }

    /// Whether it was set, clearing it.
    func take() -> Bool {
        lock.withLock {
            defer { isSet = false }
            return isSet
        }
    }
}
