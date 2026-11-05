import ComposableArchitecture
import Foundation
import LifrGlance
import OSLog
import WidgetKit

extension GlanceClient {
    /// Writes to `file` and redraws the widgets, skipping a Today that's the
    /// same as the last one so the widgets' redraw budget lasts. With no
    /// file, such as an iPhone build without an App Group, does nothing.
    package static func live(file: GlanceFile?) -> Self {
        let last = LockIsolated<TodayGlance?>(nil)
        return GlanceClient(write: { glance in
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
        })
    }
}
