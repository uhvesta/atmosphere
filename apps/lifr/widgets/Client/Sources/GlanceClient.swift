import ComposableArchitecture
import Foundation
import LifrGlance

/// Keeps the widgets' copy of Today up to date.
@DependencyClient
package struct GlanceClient: Sendable {
    /// Replaces what the widgets show with `glance`, and asks them to redraw
    /// when it changed. Failing leaves the widgets on the last Today they
    /// had, so it reports nothing to the person.
    package var write: @Sendable (_ glance: TodayGlance) async -> Void
}

extension GlanceClient {
    /// Keeps what was last written, for tests.
    package static func inMemory() -> (client: Self, written: LockIsolated<TodayGlance?>) {
        let written = LockIsolated<TodayGlance?>(nil)
        return (GlanceClient(write: { glance in written.setValue(glance) }), written)
    }
}

extension GlanceClient: TestDependencyKey {
    /// Widgets aren't part of most tests, so writing does nothing; tests of
    /// the widgets' Today use `inMemory`.
    package static let testValue = GlanceClient(write: { _ in })
    package static let previewValue = GlanceClient(write: { _ in })
}

extension DependencyValues {
    package var glance: GlanceClient {
        get { self[GlanceClient.self] }
        set { self[GlanceClient.self] = newValue }
    }
}
