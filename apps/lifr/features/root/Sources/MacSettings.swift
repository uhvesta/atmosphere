import ComposableArchitecture
import LifrSettings
import SwiftUI

/// Settings in their own window on the Mac, opened with ⌘, from the app
/// menu: the iPhone's screen split into tabs, each pushing its own screens
/// such as Activity and Skills.
package struct MacSettingsView: View {
    @Bindable package var store: StoreOf<RootFeature>

    package init(store: StoreOf<RootFeature>) {
        self.store = store
    }

    package var body: some View {
        Group {
            if store.isLocked {
                LockView(store: store)
            } else {
                tabs
            }
        }
        .formStyle(.grouped)
        .frame(minWidth: 560, idealWidth: 620, minHeight: 520, idealHeight: 680)
        .modifier(AccentTint())
    }

    private var tabs: some View {
        let settings = store.scope(state: \.settings, action: \.settings)
        return TabView(selection: Binding(get: { settings.page }, set: { settings.send(.pagePicked($0)) })) {
            tab(.general, settings, "General", systemImage: "gearshape")
            tab(.sync, settings, "Sync & Privacy", systemImage: "arrow.triangle.2.circlepath.icloud")
            tab(.assistant, settings, "Assistant", systemImage: "sparkles")
            tab(.agents, settings, "Agents", systemImage: "terminal")
        }
        // Once for every tab, so switching tabs doesn't start it again.
        .task { await settings.send(.task).finish() }
        .confirmationDialog($store.scope(state: \.settings.revoke, action: \.settings.revoke))
        .alert($store.scope(state: \.settings.alert, action: \.settings.alert))
    }

    private func tab(
        _ page: SettingsPage, _ settings: StoreOf<SettingsFeature>, _ title: LocalizedStringKey, systemImage: String
    ) -> some TabContent<SettingsPage> {
        Tab(title, systemImage: systemImage, value: page) {
            NavigationStack {
                SettingsView(store: settings, page: page)
            }
        }
    }
}
