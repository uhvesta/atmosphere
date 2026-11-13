import ComposableArchitecture
import Foundation

/// The keys that open Quick Add over any app on the Mac, or none. Off
/// until picked, since any of them may already mean something else.
package enum QuickAddHotKey: String, CaseIterable, Hashable, Sendable {
    case off
    case controlSpace
    case optionSpace
    case controlOptionSpace
    case shiftCommandSpace

    package var title: String {
        switch self {
        case .off: String(localized: "Off")
        case .controlSpace: String(localized: "⌃Space")
        case .optionSpace: String(localized: "⌥Space")
        case .controlOptionSpace: String(localized: "⌃⌥Space")
        case .shiftCommandSpace: String(localized: "⇧⌘Space")
        }
    }
}

extension SharedKey where Self == AppStorageKey<QuickAddHotKey>.Default {
    package static var quickAddHotKey: Self {
        Self[.appStorage("lifrQuickAddHotKey"), default: .off]
    }
}
