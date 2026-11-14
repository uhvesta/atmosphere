import AppKit
import ChatUI
import Observation
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
    /// Whether what's in view stays where it is by this row when rows come
    /// above it: not for a button that stays first as rows come under it.
    var holdsPlace = true
    /// The guess is a height the row was measured at, under another id.
    var measured = false
}

/// The finished parts of the reply being written, as the turn under way
/// cuts them, each to be a row of the chat of its own: so a long reply is
/// laid out a part at a time, and only those near what's in view. The
/// same for the chat's life.
@MainActor
final class StreamedRows {
    /// The parts as last told.
    private var told: [String] = []
    private var waiting = false
    fileprivate var changed: (([String]) -> Void)?

    /// The finished parts now. Told while a view is drawn, so the chat
    /// takes them just after, not in the middle of laying itself out.
    func post(_ parts: [String]) {
        guard parts != told else { return }
        told = parts
        guard !waiting else { return }
        waiting = true
        Task { @MainActor [weak self] in
            guard let self else { return }
            waiting = false
            changed?(told)
        }
    }

    /// The chat took the parts as its own rows: told again, they come back.
    fileprivate func forget() {
        told = []
    }
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
    /// The finished parts of the reply being written, rows after `rows`
    /// until the reply is among them.
    let streamed: StreamedRows
    /// Builds the row of a finished part of the reply being written, the
    /// part's place among them and its text.
    let streamedRow: @MainActor (Int, String) -> ChatWindowRow
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
        coordinator.document = document
        coordinator.row = row
        coordinator.streamedRow = streamedRow
        document.rowBuilder = { [weak coordinator] id in coordinator?.build(id) }
        coordinator.streamed = streamed
        streamed.changed = { [weak coordinator] parts in coordinator?.streamed(parts) }
        document.scrolled = moved
        let estimate = self.estimate
        document.widthChanged = { [weak document, weak coordinator] width in
            guard let document else { return }
            coordinator?.estimate(estimate, width: width, scale: bridge.textScale, into: document)
        }
        document.setInsets(top: topInset, bottom: bottomInset)
        document.setFooter(footer.map { bridge.apply(to: $0, resizing: nil) }, redraw: redraw)
        let changed = document.setRows(
            coordinator.keys(for: rows), redraw: redraw, rescaled: rescaled)
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
        fileprivate weak var document: ChatDocumentView?
        fileprivate weak var streamed: StreamedRows?
        fileprivate var row: @MainActor (String) -> ChatWindowRow? = { _ in nil }
        fileprivate var streamedRow: (@MainActor (Int, String) -> ChatWindowRow)?
        private var estimating: Task<Void, Never>?
        /// The chat's own rows, as last given.
        private var lines: [ChatWindowKey] = []
        /// The finished parts of the reply being written, rows after them.
        private var parts: [String] = []
        /// Counts the replies written, so one's parts aren't taken for
        /// another's.
        private var reply = 0
        /// The heights the last reply's parts were measured at, for the
        /// reply's own rows once they come.
        private var handed: [ChatRowLayout.Row] = []

        private static let streamPrefix = "stream-"

        /// Builds the row `id`: a part of the reply being written, or one
        /// of the chat's, whose reads of what it shows are watched, so it's
        /// built again when they change.
        fileprivate func build(_ id: String) -> ChatWindowRow? {
            if let index = streamIndex(id) {
                guard index < parts.count, let streamedRow else { return nil }
                return streamedRow(index, parts[index])
            }
            let row = self.row
            return withObservationTracking {
                row(id)
            } onChange: { [weak self] in
                Task { @MainActor [weak self] in self?.document?.reread() }
            }
        }

        /// The rows to show: the chat's `lines`, then the parts of the
        /// reply being written. Once the reply is among `lines`, its rows
        /// take the parts' place and the heights they were measured at.
        fileprivate func keys(for lines: [ChatWindowKey]) -> [ChatWindowKey] {
            let old = self.lines
            self.lines = lines
            let grew = lines.count > old.count && zip(old, lines).allSatisfy { $0.id == $1.id }
            guard grew else {
                // Heights kept for a reply that came as something else.
                if lines.map(\.id) != old.map(\.id) { handed = [] }
                return lines + streamKeys
            }
            if !parts.isEmpty {
                handed = streamHeights()
                parts = []
                reply += 1
                streamed?.forget()
            }
            guard !handed.isEmpty else { return lines }
            defer { handed = [] }
            // A reply's rows are its id, then the id and /1, /2 and on, as
            // `Chat` cuts it.
            var keys = lines
            let first = old.count
            let replyID = keys[first].id
            // Any row's id is its first part's: only a second part named
            // after it shows the rows are a reply's, and their heights its
            // own. Otherwise they're guesses, measured again when hosted.
            let isReply = first + 1 < keys.count && keys[first + 1].id == "\(replyID)/1"
            for (offset, part) in handed.enumerated() {
                let index = first + offset
                guard index < keys.count, keys[index].id == (offset == 0 ? replyID : "\(replyID)/\(offset)")
                else { break }
                let key = keys[index]
                keys[index] = ChatWindowKey(
                    id: key.id, version: key.version, guess: part.height, holdsPlace: key.holdsPlace,
                    measured: part.measured && isReply)
            }
            return keys
        }

        /// The reply being written has these finished parts now.
        fileprivate func streamed(_ new: [String]) {
            guard new != parts else { return }
            if new.isEmpty {
                // Gone before the reply came as a row: its rows, if it
                // comes, start at these heights, but as guesses, since
                // what comes may not be the reply.
                handed = streamHeights().map {
                    ChatRowLayout.Row(id: $0.id, height: $0.height, measured: false)
                }
                reply += 1
            } else if parts.isEmpty {
                // A reply after the last.
                handed = []
            } else if new.count < parts.count || zip(parts, new).contains(where: { $0 != $1 }) {
                // Another reply.
                handed = []
                reply += 1
            }
            parts = new
            document?.setRows(lines + streamKeys, redraw: false, rescaled: false)
        }

        private var streamKeys: [ChatWindowKey] {
            parts.indices.map { ChatWindowKey(id: "\(Self.streamPrefix)\(reply)-\($0)", version: 0, guess: 80) }
        }

        /// The place among the parts of the row `id`, if it's one of the
        /// reply being written now.
        private func streamIndex(_ id: String) -> Int? {
            let prefix = "\(Self.streamPrefix)\(reply)-"
            guard id.hasPrefix(prefix) else { return nil }
            return Int(id.dropFirst(prefix.count))
        }

        /// The heights the parts' rows have, measured or not.
        private func streamHeights() -> [ChatRowLayout.Row] {
            guard let document else { return [] }
            let layout = document.rows
            return streamKeys.compactMap { key in
                layout.position(of: key.id).map {
                    ChatRowLayout.Row(id: key.id, height: layout.heights[$0], measured: layout.measured[$0])
                }
            }
        }

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
    let font: Font?

    init(_ environment: EnvironmentValues) {
        font = environment.font
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
            && lhs.font == rhs.font
            && (lhs.codeBlockRenderer == nil) == (rhs.codeBlockRenderer == nil)
            && (lhs.codeQuoter == nil) == (rhs.codeQuoter == nil)
    }

    func apply(to view: AnyView, resizing: RowResizing?) -> AnyView {
        AnyView(
            view
                .environment(\.font, font)
                .environment(\.textScale, textScale)
                .environment(\.colorScheme, colorScheme)
                .environment(\.isEnabled, isEnabled)
                .environment(\.codeBlockRenderer, codeBlockRenderer)
                .environment(\.codeQuoter, codeQuoter)
                .environment(\.rowResizing, resizing))
    }
}
