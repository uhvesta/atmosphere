import ComposableArchitecture
import Foundation

/// Where the person was in Lifr: the tab or sidebar item and the open task,
/// so Lifr opens there again after the system ends it in the background.
package struct Place: Codable, Hashable, Sendable {
    /// The Mac's other windows, each opening at a place of its own.
    package static let windowID = "window"

    package var tab: RootFeature.Tab?
    package var sidebar: RootFeature.SidebarItem?
    package var task: UUID?

    package init(tab: RootFeature.Tab? = nil, sidebar: RootFeature.SidebarItem? = nil, task: UUID? = nil) {
        self.tab = tab
        self.sidebar = sidebar
        self.task = task
    }
}

extension SharedKey where Self == FileStorageKey<Place>.Default {
    /// Where the person was, on this device.
    package static var place: Self {
        Self[.fileStorage(.applicationSupportDirectory.appending(path: "Lifr/place.json")), default: Place()]
    }
}
