import ChatUI
import ComposableArchitecture
import HumataBoard
import HumataDiagram
import HumataHosts
import HumataProjects
import HumataSearch
import HumataSession
import SwiftUI

/// The sidebar and the open session's chat, with the daemon's state above
/// them when it needs the person.
package struct AppView: View {
    @Bindable var store: StoreOf<AppFeature>

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
            // The column's own window view asks its content for its smallest
            // size on every change; this answers without measuring the chat.
            AtLeast(width: 360, height: 300) { detail }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            DaemonBanner(store: store)
        }
        .alert($store.scope(state: \.sendFailure, action: \.sendFailure))
        // Mermaid blocks in chats drawn as diagrams.
        .environment(\.codeBlockRenderer, MermaidRenderer.codeBlocks)
        .task {
            store.send(.task)
            // Not awaited, so the sidebar's poll, its notices and the Dock
            // badge go on while the window is closed.
            store.send(.sidebar(.task))
        }
    }

    @ViewBuilder private var detail: some View {
        if store.sidebar.selection == .inbox {
            InboxView(store: store.scope(state: \.sidebar, action: \.sidebar))
        } else if let search = store.scope(state: \.search, action: \.search.presented) {
            SearchView(store: search)
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
}

/// Takes the space it's offered, at least `width` by `height`, without
/// asking its view. A window view asks its content for its smallest size on
/// every change; with this as the content, that's this answer, not a measure
/// of the whole chat each time a reply grows.
package struct AtLeast<Content: View>: View {
    let width: CGFloat
    let height: CGFloat
    @ViewBuilder let content: () -> Content

    package init(width: CGFloat, height: CGFloat, @ViewBuilder content: @escaping () -> Content) {
        self.width = width
        self.height = height
        self.content = content
    }

    package var body: some View {
        AtLeastLayout(width: width, height: height) { content() }
    }
}

private struct AtLeastLayout: Layout {
    let width: CGFloat
    let height: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        CGSize(width: max(proposal.width ?? width, width), height: max(proposal.height ?? height, height))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for view in subviews {
            view.place(at: bounds.origin, proposal: ProposedViewSize(bounds.size))
        }
    }

    // Without these, a stack asking where this view's alignment guides are
    // has it place its view to find out, laying out all of it.
    func explicitAlignment(
        of guide: HorizontalAlignment, in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews,
        cache: inout ()
    ) -> CGFloat? { nil }

    func explicitAlignment(
        of guide: VerticalAlignment, in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews,
        cache: inout ()
    ) -> CGFloat? { nil }
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
