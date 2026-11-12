import ComposableArchitecture
import HumataBoard
import HumataHosts
import HumataProjects
import HumataSession
import SwiftUI

/// The sidebar and the open session's chat, with the daemon's state above
/// them when it needs the person.
package struct AppView: View {
    let store: StoreOf<AppFeature>

    package init(store: StoreOf<AppFeature>) {
        self.store = store
    }

    package var body: some View {
        NavigationSplitView {
            Group {
                if store.daemon == .starting {
                    ProgressView("Starting Humata's daemon…")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ProjectsView(store: store.scope(state: \.sidebar, action: \.sidebar))
                }
            }
            .navigationSplitViewColumnWidth(min: 220, ideal: 260)
        } detail: {
            if store.sidebar.selection == .inbox {
                InboxView(entries: store.sidebar.inbox) {
                    store.send(.sidebar(.selected(.session($0))))
                } answer: { session, request, approved in
                    store.send(.sidebar(.inboxAnswered(session, request: request, approved: approved)))
                }
            } else if let hosts = store.scope(state: \.hosts, action: \.hosts.presented) {
                HostsView(store: hosts)
            } else if let board = store.scope(state: \.board, action: \.board.presented) {
                NavigationStack {
                    BoardView(store: board)
                }
                .id(board.project.id)
            } else if let session = store.scope(state: \.session, action: \.session.presented) {
                SessionView(store: session)
                    .id(session.session.id)
            } else {
                ContentUnavailableView(
                    "No Session Open", systemImage: "bubble.left.and.text.bubble.right",
                    description: Text("Pick a session in the sidebar, or start one in a project."))
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            DaemonBanner(store: store)
        }
        .task { store.send(.task) }
    }
}

/// What the daemon needs from the person, if anything.
private struct DaemonBanner: View {
    let store: StoreOf<AppFeature>

    var body: some View {
        switch store.daemon {
        case .starting, .running:
            EmptyView()
        case .updatePending:
            banner(
                "Humata was updated. Its daemon restarts when no session is working or waiting; restarting now "
                    + "stops their turns.",
                systemImage: "arrow.triangle.2.circlepath"
            ) {
                Button("Restart Now") { store.send(.restartTapped) }
            }
        case .restarting:
            banner("Restarting Humata's daemon…", systemImage: "arrow.triangle.2.circlepath") {
                ProgressView().controlSize(.small)
            }
        case .failed(let message, let needsLoginItems):
            banner(message, systemImage: "exclamationmark.triangle.fill") {
                if needsLoginItems {
                    Button("Open Login Items") { store.send(.openLoginItemsTapped) }
                }
                Button("Try Again") { store.send(.task) }
            }
        }
    }

    private func banner(
        _ text: String, systemImage: String, @ViewBuilder actions: () -> some View
    ) -> some View {
        HStack(spacing: 10) {
            Image(systemName: systemImage)
                .foregroundStyle(.secondary)
            Text(text)
                .font(.callout)
                .fixedSize(horizontal: false, vertical: true)
            Spacer()
            actions()
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(.bar)
        .overlay(alignment: .bottom) { Divider() }
    }
}
