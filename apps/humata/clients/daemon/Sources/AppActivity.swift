import ComposableArchitecture
import Foundation

/// Whether Humata is the app in front, and its Dock icon. A reply counts as
/// seen, and goes without a notice, only while the person looks at Humata.
package struct AppActivity: Sendable {
    /// True while Humata is the active app.
    package var isActive: @Sendable () -> Bool
    /// Each time Humata comes to the front.
    package var activations: @Sendable () -> AsyncStream<Void>
    /// Shows `count` on the Dock icon; nothing when it's zero.
    package var setBadge: @Sendable (_ count: Int) -> Void

    package init(
        isActive: @escaping @Sendable () -> Bool,
        activations: @escaping @Sendable () -> AsyncStream<Void>,
        setBadge: @escaping @Sendable (_ count: Int) -> Void
    ) {
        self.isActive = isActive
        self.activations = activations
        self.setBadge = setBadge
    }
}

extension AppActivity: TestDependencyKey {
    /// Always in front, with no Dock icon.
    package static let testValue = AppActivity(
        isActive: { true }, activations: { AsyncStream { $0.finish() } }, setBadge: { _ in })
}

extension DependencyValues {
    package var appActivity: AppActivity {
        get { self[AppActivity.self] }
        set { self[AppActivity.self] = newValue }
    }
}
