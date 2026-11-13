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
    /// The tasks marked Done from a widget since Lifr last took them,
    /// leaving none marked.
    package var takeMarked: @Sendable () async throws -> Set<UUID>
}

extension GlanceClient {
    /// Keeps what was last written, for tests.
    package static func inMemory(
        marked: Set<UUID> = []
    ) -> (client: Self, written: LockIsolated<TodayGlance?>) {
        let written = LockIsolated<TodayGlance?>(nil)
        let marks = LockIsolated(marked)
        return (
            GlanceClient(
                write: { glance in written.setValue(glance) },
                takeMarked: {
                    marks.withValue { marked in
                        defer { marked = [] }
                        return marked
                    }
                }),
            written
        )
    }
}

extension GlanceClient: TestDependencyKey {
    /// Widgets aren't part of most tests, so writing does nothing; tests of
    /// the widgets' Today use `inMemory`.
    package static let testValue = GlanceClient(write: { _ in }, takeMarked: { [] })
    package static let previewValue = GlanceClient(write: { _ in }, takeMarked: { [] })
}

extension DependencyValues {
    package var glance: GlanceClient {
        get { self[GlanceClient.self] }
        set { self[GlanceClient.self] = newValue }
    }
}
