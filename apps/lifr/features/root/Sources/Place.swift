import ComposableArchitecture
import Foundation

/// Where the person was in Lifr: the tab or sidebar item and the open task,
/// so Lifr opens there again after the system ends it in the background.
package struct Place: Codable, Hashable, Sendable {
    package var tab: RootFeature.Tab?
    package var sidebar: RootFeature.SidebarItem?
    package var task: UUID?

    package init(tab: RootFeature.Tab? = nil, sidebar: RootFeature.SidebarItem? = nil, task: UUID? = nil) {
        self.tab = tab
        self.sidebar = sidebar
        self.task = task
    }
}

/// One of the Mac's other windows: where it is, and which window it is,
/// so two windows at the same place are still two.
package struct WindowPlace: Codable, Hashable, Sendable {
    /// The scene the Mac's other windows open in.
    package static let sceneID = "window"

    package let id: UUID
    package var place: Place

    package init(_ place: Place) {
        id = UUID()
        self.place = place
    }
}

extension SharedKey where Self == FileStorageKey<Place>.Default {
    /// Where the person was, on this device.
    package static var place: Self {
        Self[.fileStorage(.applicationSupportDirectory.appending(path: "Lifr/place.json")), default: Place()]
    }
}
