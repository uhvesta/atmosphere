import ChatUI
import ComposableArchitecture
import HumataBoard
import HumataDiagram
import HumataDocs
import HumataHosts
import HumataProjects
import HumataSearch
import HumataSession
import SwiftUI

/// The sidebar and the open session's chat, with the daemon's state above
/// them when it needs the person.
package struct AppView: View {
    @Bindable var store: StoreOf<AppFeature>
    /// The chat column the keyboard is in; nil when it's in neither.
    @FocusedValue(\.sessionColumn) private var column

    package init(store: StoreOf<AppFeature>) {
        self.store = store
    }

    /// Whether the open session shows a document with its chat beside it,
    /// which needs more room than the chat alone: the window then asks the
    /// session how wide it must be instead of answering its fixed least.
    private var focusesDocument: Bool {
        guard let session = store.session else { return false }
        switch session.tab {
        case .files: return session.docs.focused
        case .spec: return session.specs.focused != nil
        default: return false
        }
    }

    package var body: some View {
        // The window's own view asks its content for its smallest size on
        // every change; this answers without measuring the chat, and is here
        // so that whatever hosts this view lays out what the app does.
        AtLeast(width: 820, height: 520, fits: focusesDocument) { split }
    }

    private var split: some View {
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
            AtLeast(width: 360, height: 300, fits: focusesDocument) { detail }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            DaemonBanner(store: store)
        }
        .alert($store.scope(state: \.sendFailure, action: \.sendFailure))
        // Mermaid blocks in chats drawn as diagrams.
        .environment(\.codeBlockRenderer, MermaidRenderer.codeBlocks)
        // The Session menu and the chat's shortcuts follow the column.
        .onChange(of: column) { store.send(.columnFocused(column)) }
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
            // The session keeps its place whether or not a side chat is
            // beside it, so opening or closing one doesn't draw it anew.
            HStack(spacing: 0) {
                AtLeast(width: 360, height: 300, fits: focusesDocument) {
                    SessionView(store: session)
                        .id(session.session.id)
                }
                .focusedValue(\.sessionColumn, .main)
                if let side = store.scope(state: \.side, action: \.side.presented) {
                    SideChatColumn(store: side) { store.send(.side(.dismiss)) }
                        .focusedValue(\.sessionColumn, .side)
                }
            }
        } else {
            ContentUnavailableView(
                "No Session Open", systemImage: "bubble.left.and.text.bubble.right",
                description: Text("Pick a session in the sidebar, or start one in a project."))
        }
    }
}

extension FocusedValues {
    /// Which of the window's two chats the keyboard is in.
    @Entry fileprivate var sessionColumn: AppFeature.SessionColumn?
}

/// A side chat beside the open session: a bar that says what it is and
/// closes it, over its chat, as wide as it's dragged and as wide the next
/// time one opens.
private struct SideChatColumn: View {
    let store: StoreOf<SessionFeature>
    let close: () -> Void
    /// The width it was last left at, kept between runs.
    @AppStorage("sideChatWidth") private var keptWidth = 440.0
    /// The width while it's dragged; nil between drags, so only the end
    /// of a drag is written down.
    @State private var dragged: CGFloat?
    /// The width when a drag began; nil between drags.
    @State private var dragFrom: CGFloat?

    private static let widths: ClosedRange<CGFloat> = 360...900

    /// What's kept may be from another build, or edited: held to the range.
    private var width: CGFloat {
        dragged ?? min(max(CGFloat(keptWidth), Self.widths.lowerBound), Self.widths.upperBound)
    }

    var body: some View {
        HStack(spacing: 0) {
            Divider()
                .overlay {
                    // Wider than the line, to be caught.
                    Color.clear
                        .frame(width: 8)
                        .contentShape(.rect)
                        .gesture(
                            DragGesture(minimumDistance: 1, coordinateSpace: .global)
                                .onChanged { drag in
                                    let from = dragFrom ?? width
                                    dragFrom = from
                                    let next = from - drag.translation.width
                                    dragged = min(max(next, Self.widths.lowerBound), Self.widths.upperBound)
                                }
                                .onEnded { _ in
                                    if let dragged { keptWidth = Double(dragged) }
                                    dragged = nil
                                    dragFrom = nil
                                }
                        )
                        .help("Drag to make the side chat wider or narrower")
                }
            AtLeast(width: Self.widths.lowerBound, height: 300) {
                VStack(spacing: 0) {
                    HStack(spacing: 8) {
                        Label("Side Chat", systemImage: "rectangle.righthalf.inset.filled")
                            .font(.callout.weight(.medium))
                        Text("Reads only")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Spacer(minLength: 0)
                        Button("Close", systemImage: "xmark", action: close)
                            .labelStyle(.iconOnly)
                            .buttonStyle(.borderless)
                            .help("Close the side chat; it stays under its session in the sidebar (⌥⌘W)")
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(.bar)
                    Divider()
                    SessionView(store: store)
                        .id(store.session.id)
                }
            }
            .frame(width: width)
        }
    }
}

/// Takes the space it's offered, at least `width` by `height`, without
/// asking its view. A window view asks its content for its smallest size on
/// every change; with this as the content, that's this answer, not a measure
/// of the whole chat each time a reply grows. With `fits`, it is also at
/// least as wide as its view says it must be at that least width, for a view
/// that needs more than the least, as a document with its chat beside it
/// does; the height is never asked.
package struct AtLeast<Content: View>: View {
    let width: CGFloat
    let height: CGFloat
    let fits: Bool
    @ViewBuilder let content: () -> Content

    package init(
        width: CGFloat, height: CGFloat, fits: Bool = false, @ViewBuilder content: @escaping () -> Content
    ) {
        self.width = width
        self.height = height
        self.fits = fits
        self.content = content
    }

    package var body: some View {
        AtLeastLayout(width: width, height: height, fits: fits) { content() }
    }
}

private struct AtLeastLayout: Layout {
    let width: CGFloat
    let height: CGFloat
    let fits: Bool

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let tall = max(proposal.height ?? height, height)
        var wide = max(proposal.width ?? width, width)
        if fits {
            // What its view answers when offered the least width is the
            // width it can't go under.
            for view in subviews {
                wide = max(wide, view.sizeThatFits(ProposedViewSize(width: width, height: tall)).width)
            }
        }
        return CGSize(width: wide, height: tall)
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
