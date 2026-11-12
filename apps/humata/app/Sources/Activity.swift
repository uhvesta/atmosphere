import AppKit
import ComposableArchitecture
import HumataDaemonClient

extension AppActivity {
    /// NSApplication's: whether it's active, as its notifications say, and
    /// its Dock tile.
    @MainActor static func live() -> AppActivity {
        let active = LockIsolated(NSApplication.shared.isActive)
        let center = NotificationCenter.default
        center.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: nil) { _ in
            active.setValue(true)
        }
        center.addObserver(forName: NSApplication.didResignActiveNotification, object: nil, queue: nil) { _ in
            active.setValue(false)
        }
        return AppActivity(
            isActive: { active.value },
            activations: {
                AsyncStream { continuation in
                    let observer = center.addObserver(
                        forName: NSApplication.didBecomeActiveNotification, object: nil, queue: nil
                    ) { _ in continuation.yield() }
                    // The token isn't Sendable; the center takes it back.
                    let token = UncheckedSendable(observer)
                    continuation.onTermination = { _ in center.removeObserver(token.value) }
                }
            },
            setBadge: { count in
                Task { @MainActor in NSApp?.dockTile.badgeLabel = count == 0 ? nil : String(count) }
            }
        )
    }
}
