import AppKit
import ChatUI
import SwiftUI

/// A row of the chat as the window tells them apart: its id, the version
/// of what it shows, and the height it's given until it's measured or
/// guessed at.
struct ChatWindowKey: Equatable, Sendable {
    let id: String
    /// Moves on when what the row shows changes by itself, as a tool call
    /// finishing.
    let version: UInt32
    /// The row's kind's usual height, until a better guess comes.
    let guess: CGFloat
}

/// What the chat's view asks of the scrolling: to take the chat to a row
/// or to its end. The same for the chat's life; it reaches the scroll view
/// once that's made, and keeps what's asked until then.
@MainActor
final class ChatScrolling {
    private weak var document: ChatDocumentView?
    private var waiting: Request?

    private enum Request {
        case row(id: String, placement: ChatScrollPlacement, animated: Bool)
        case end
    }

    /// Whether the chat keeps to its end as it grows.
    var isFollowing: Bool { document?.following ?? true }

    /// How far above its end the chat is scrolled.
    var aboveEnd: CGFloat { document?.aboveEnd ?? 0 }

    func scroll(to id: String, placement: ChatScrollPlacement, animated: Bool) {
        guard let document else {
            waiting = .row(id: id, placement: placement, animated: animated)
            return
        }
        document.scroll(to: id, placement: placement, animated: animated)
    }

    func scrollToEnd() {
        guard let document else {
            waiting = .end
            return
        }
        document.scrollToEnd()
    }

    fileprivate func attach(_ document: ChatDocumentView) {
        guard self.document !== document else { return }
        self.document = document
        switch waiting {
        case .row(let id, let placement, let animated):
            document.scroll(to: id, placement: placement, animated: animated)
        case .end:
            document.scrollToEnd()
        case nil:
            break
        }
        waiting = nil
    }
}

/// The chat's rows in a scroll view that lays them out itself, hosting
/// only those near what's in view. Rows are built by `row` when hosted,
/// and again only when their key or what they draw changes.
struct ChatWindow: NSViewRepresentable {
    /// The rows, in order.
    let rows: [ChatWindowKey]
    /// Builds the row with an id as it shows now; nil for none.
    let row: @MainActor (String) -> ChatWindowRow?
    /// Under the last row: the turn under way. Drawn again only when the
    /// environment it's drawn in changes; it watches what it shows itself.
    var footer: AnyView?
    /// About how tall each row is at a width and text scale. Run off the
    /// main thread when the rows or the width change.
    let estimate: @Sendable (_ width: CGFloat, _ scale: TextScale) -> [String: CGFloat]
    /// Room over and under the rows for what's drawn over the chat.
    var topInset: CGFloat = 0
    var bottomInset: CGFloat = 0
    let scrolling: ChatScrolling
    /// The chat moved under the pointer.
    var moved: @MainActor () -> Void = {}

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> ChatScrollView {
        ChatScrollView(document: ChatDocumentView())
    }

    func updateNSView(_ scroll: ChatScrollView, context: Context) {
        let document = scroll.document
        let coordinator = context.coordinator
        let bridge = EnvironmentBridge(context.environment)
        let redraw = coordinator.bridge.map { $0 != bridge } ?? false
        let rescaled = coordinator.bridge.map { $0.textScale != bridge.textScale } ?? false
        coordinator.bridge = bridge
        let appearance: NSAppearance.Name = bridge.colorScheme == .dark ? .darkAqua : .aqua
        if scroll.appearance?.name != appearance { scroll.appearance = NSAppearance(named: appearance) }
        let list = ObjectIdentifier(document)
        document.wrap = { [weak document] id, view in
            let resizing = RowResizing(row: id, list: list) { [weak document] in document?.invalidate($0) }
            return bridge.apply(to: view, resizing: resizing)
        }
        document.rowBuilder = row
        document.scrolled = moved
        let estimate = self.estimate
        document.widthChanged = { [weak document, weak coordinator] width in
            guard let document else { return }
            coordinator?.estimate(estimate, width: width, scale: bridge.textScale, into: document)
        }
        document.setInsets(top: topInset, bottom: bottomInset)
        document.setFooter(footer.map { bridge.apply(to: $0, resizing: nil) }, redraw: redraw)
        let changed = document.setRows(rows, redraw: redraw, rescaled: rescaled)
        if changed || rescaled, document.measuredWidth > 0 {
            coordinator.estimate(estimate, width: document.measuredWidth, scale: bridge.textScale, into: document)
        }
        scrolling.attach(document)
    }

    static func dismantleNSView(_ scroll: ChatScrollView, coordinator: Coordinator) {
        coordinator.stop()
    }

    @MainActor
    final class Coordinator {
        fileprivate var bridge: EnvironmentBridge?
        private var estimating: Task<Void, Never>?

        /// Works out guesses for the rows off the main thread, the last
        /// asked for replacing any still under way.
        func estimate(
            _ estimate: @escaping @Sendable (CGFloat, TextScale) -> [String: CGFloat], width: CGFloat,
            scale: TextScale, into document: ChatDocumentView
        ) {
            estimating?.cancel()
            estimating = Task { [weak document] in
                let heights = await Task.detached(priority: .userInitiated) { estimate(width, scale) }.value
                guard !Task.isCancelled else { return }
                document?.takeEstimates(heights)
            }
        }

        func stop() {
            estimating?.cancel()
            estimating = nil
        }
    }
}

/// What a hosted row needs of the chat's environment, which doesn't reach
/// into a hosting view: read where the chat is, put around each row.
private struct EnvironmentBridge: Equatable {
    let textScale: TextScale
    let colorScheme: ColorScheme
    let isEnabled: Bool
    let codeBlockRenderer: CodeBlockRenderer?
    let codeQuoter: CodeQuoter?

    init(_ environment: EnvironmentValues) {
        textScale = environment.textScale
        colorScheme = environment.colorScheme
        isEnabled = environment.isEnabled
        codeBlockRenderer = environment.codeBlockRenderer
        codeQuoter = environment.codeQuoter
    }

    /// Equal when rows drawn in either look the same. A renderer or quoter
    /// counts only as there or not: the chat's are the same for its life.
    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.textScale == rhs.textScale && lhs.colorScheme == rhs.colorScheme && lhs.isEnabled == rhs.isEnabled
            && (lhs.codeBlockRenderer == nil) == (rhs.codeBlockRenderer == nil)
            && (lhs.codeQuoter == nil) == (rhs.codeQuoter == nil)
    }

    func apply(to view: AnyView, resizing: RowResizing?) -> AnyView {
        AnyView(
            view
                .environment(\.textScale, textScale)
                .environment(\.colorScheme, colorScheme)
                .environment(\.isEnabled, isEnabled)
                .environment(\.codeBlockRenderer, codeBlockRenderer)
                .environment(\.codeQuoter, codeQuoter)
                .environment(\.rowResizing, resizing))
    }
}
