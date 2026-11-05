import ComposableArchitecture
import LifrSettings
import SwiftUI

/// Settings in their own window on the Mac, opened with ⌘, from the app
/// menu: the same screen the iPhone shows as a sheet, with Activity and
/// Skills pushed inside it.
package struct MacSettingsView: View {
    package var store: StoreOf<RootFeature>

    package init(store: StoreOf<RootFeature>) {
        self.store = store
    }

    package var body: some View {
        Group {
            if store.isLocked {
                LockView(store: store)
            } else {
                NavigationStack {
                    SettingsView(store: store.scope(state: \.settings, action: \.settings))
                }
            }
        }
        .formStyle(.grouped)
        .frame(minWidth: 560, idealWidth: 620, minHeight: 520, idealHeight: 680)
    }
}
