import ComposableArchitecture
import Foundation
import LifrCore

extension DefaultReminder {
    package var title: String {
        switch self {
        case .never: String(localized: "Never")
        case .atDue: String(localized: "At the due time")
        case .dueDayMorning: String(localized: "At 9:00 on the due day")
        }
    }
}

extension SharedKey where Self == AppStorageKey<DefaultReminder>.Default {
    /// This device's copy of `Settings.defaultReminder`, so Quick Add,
    /// widgets and Shortcuts read it without the store. Off until picked.
    package static var defaultReminder: Self {
        Self[.appStorage("lifrDefaultReminder"), default: .never]
    }
}
