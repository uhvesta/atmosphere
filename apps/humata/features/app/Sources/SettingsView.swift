import ComposableArchitecture
import HumataHosts
import HumataProjects
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
            ApprovalsView(store: store.scope(state: \.approvals, action: \.approvals))
                .tabItem { Label("Approvals", systemImage: "lock.shield") }
        }
        .frame(minWidth: 760, minHeight: 480)
    }
}
