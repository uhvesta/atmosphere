import ComposableArchitecture
import HumataHosts
import SwiftUI

/// Humata's Settings window (⌘,).
package struct SettingsView: View {
    let store: StoreOf<AppFeature>

    package init(store: StoreOf<AppFeature>) {
        self.store = store
    }

    package var body: some View {
        TabView {
            EnvironmentsView(store: store.scope(state: \.settings, action: \.settings))
                .tabItem { Label("Environments", systemImage: "server.rack") }
        }
        .frame(minWidth: 760, minHeight: 480)
    }
}
