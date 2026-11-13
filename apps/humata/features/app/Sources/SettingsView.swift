import ComposableArchitecture
import HumataHosts
import HumataProjects
import HumataTerminal
import SwiftUI

/// Humata's Settings window (⌘,).
package struct SettingsView: View {
    let store: StoreOf<AppFeature>

    package init(store: StoreOf<AppFeature>) {
        self.store = store
    }

    @AppStorage(SettingsTab.key) private var tab = SettingsTab.environments

    package var body: some View {
        TabView(selection: $tab) {
            EnvironmentsView(store: store.scope(state: \.settings, action: \.settings))
                .tabItem { Label("Environments", systemImage: "server.rack") }
                .tag(SettingsTab.environments)
            ApprovalsView(store: store.scope(state: \.approvals, action: \.approvals))
                .tabItem { Label("Approvals", systemImage: "lock.shield") }
                .tag(SettingsTab.approvals)
            StorageView(store: store.scope(state: \.storage, action: \.storage))
                .tabItem { Label("Storage", systemImage: "internaldrive") }
                .tag(SettingsTab.storage)
            SkillsView(store: store.scope(state: \.skills, action: \.skills))
                .tabItem { Label("Skills", systemImage: "books.vertical") }
                .tag(SettingsTab.skills)
            PluginsView(store: store.scope(state: \.plugins, action: \.plugins))
                .tabItem { Label("Plugins", systemImage: "puzzlepiece.extension") }
                .tag(SettingsTab.plugins)
            ModelDefaultsView(store: store.scope(state: \.models, action: \.models))
                .tabItem { Label("Models", systemImage: "cpu") }
                .tag(SettingsTab.models)
            TerminalSettingsView()
                .tabItem { Label("Terminal", systemImage: "terminal") }
                .tag(SettingsTab.terminal)
        }
        .frame(minWidth: 760, minHeight: 480)
    }
}
