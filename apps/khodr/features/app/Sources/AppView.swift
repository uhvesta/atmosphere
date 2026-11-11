import ComposableArchitecture
import KhodrProjects
import KhodrSession
import SwiftUI

/// The sidebar and the open session's chat.
package struct AppView: View {
    let store: StoreOf<AppFeature>

    package init(store: StoreOf<AppFeature>) {
        self.store = store
    }

    package var body: some View {
        NavigationSplitView {
            ProjectsView(store: store.scope(state: \.sidebar, action: \.sidebar))
                .navigationSplitViewColumnWidth(min: 220, ideal: 260)
        } detail: {
            if let session = store.scope(state: \.session, action: \.session.presented) {
                SessionView(store: session)
                    .id(session.session.id)
            } else {
                ContentUnavailableView(
                    "No Session Open", systemImage: "bubble.left.and.text.bubble.right",
                    description: Text("Pick a session in the sidebar, or start one in a project."))
            }
        }
    }
}
