import ComposableArchitecture
import Foundation
import LifrGlance
import OSLog
import WidgetKit

extension GlanceClient {
    /// Writes to `file` and redraws the widgets, skipping a Today that's the
    /// same as the last one so the widgets' redraw budget lasts. With no
    /// file, such as an iPhone build without an App Group, does nothing.
    /// Takes what the widgets marked Done from `marks`.
    /// Takes what was added from the share sheet from `shared`.
    package static func live(
        file: GlanceFile?, marks: MarksFile? = MarksFile.shared, shared: SharedTasksFile? = SharedTasksFile.shared
    ) -> Self {
        let last = LockIsolated<TodayGlance?>(nil)
        let write: @Sendable (TodayGlance) async -> Void = { glance in
            guard let file else { return }
            var same = glance
            same.asOf = last.value?.asOf ?? glance.asOf
            guard same != last.value else { return }
            do throws(GlanceError) {
                try file.write(glance)
            } catch {
                Logger(subsystem: "lifr", category: "widgets").error("\(error.description, privacy: .public)")
                return
            }
            last.setValue(glance)
            WidgetCenter.shared.reloadAllTimelines()
        }
        let take: @Sendable () async throws -> Set<UUID> = {
            guard let marks else { return [] }
            let marked = try marks.take()
            // The widgets stop striking them through, whether or not the
            // saves change Today.
            if !marked.isEmpty { WidgetCenter.shared.reloadAllTimelines() }
            return marked
        }
        return GlanceClient(write: write, takeMarked: take, takeShared: { try shared?.take() ?? [] })
    }
}
