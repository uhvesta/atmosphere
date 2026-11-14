import AppKit
import SwiftUI

/// Where a row the chat is taken to sits in the pane.
enum ChatScrollPlacement: Sendable {
    case top
    case center
}

/// One of the chat's rows as the scroll view hosts it: what it draws, and
/// a value equal for two rows that draw the same, so a host keeps what it
/// shows when the row comes again unchanged.
struct ChatWindowRow {
    let view: AnyView
    let value: any Equatable

    init<Value: Equatable, Content: View>(value: Value, @ViewBuilder content: () -> Content) {
        self.value = value
        view = AnyView(content())
    }

    fileprivate init(view: AnyView, value: any Equatable) {
        self.view = view
        self.value = value
    }

    /// Whether this row draws what `other` does.
    fileprivate func drawsSame(as other: (any Equatable)?) -> Bool {
        guard let other else { return false }
        func equals<Value: Equatable>(_ mine: Value) -> Bool { (other as? Value) == mine }
        return equals(value)
    }
}

/// The chat's scroll view: its document lays out the rows itself, from
/// heights it keeps, and hosts only those in view and a screen either side.
@MainActor
final class ChatScrollView: NSScrollView {
    let document: ChatDocumentView

    init(document: ChatDocumentView) {
        self.document = document
        super.init(frame: .zero)
        hasVerticalScroller = true
        hasHorizontalScroller = false
        drawsBackground = false
        borderType = .noBorder
        automaticallyAdjustsContentInsets = false
        contentView.postsBoundsChangedNotifications = true
        documentView = document
        let center = NotificationCenter.default
        center.addObserver(
            self, selector: #selector(clipMoved), name: NSView.boundsDidChangeNotification, object: contentView)
        center.addObserver(
            self, selector: #selector(liveScrollStarted), name: NSScrollView.willStartLiveScrollNotification,
            object: self)
        center.addObserver(
            self, selector: #selector(liveScrollEnded), name: NSScrollView.didEndLiveScrollNotification, object: self)
    }

    required init?(coder: NSCoder) { nil }

    override func tile() {
        super.tile()
        // A new size, or scroll bars of another style: the rows are laid
        // out again for the clip view's width.
        document.needsLayout = true
    }

    override func scrollWheel(with event: NSEvent) {
        super.scrollWheel(with: event)
        // A mouse wheel has no phases, and no live scroll: where it leaves
        // the chat says whether it keeps to its end.
        if event.phase.isEmpty, event.momentumPhase.isEmpty { document.following = document.isAtEnd }
    }

    @objc private func clipMoved(_ notification: Notification) { document.clipMoved() }

    @objc private func liveScrollStarted(_ notification: Notification) { document.handStarted() }

    @objc private func liveScrollEnded(_ notification: Notification) { document.handEnded() }
}

/// The chat's rows, laid out by `ChatRowLayout` and hosted a row a view.
/// A change only marks it as needing layout, so many in one frame cost one
/// layout; each layout keeps what was in view where it was, in the same
/// pass, before anything is drawn.
@MainActor
final class ChatDocumentView: NSView {
    /// Whether the chat keeps to its end as it grows.
    var following = true
    /// Builds the row with an id as it shows now; nil for one it doesn't hold.
    var rowBuilder: (String) -> ChatWindowRow? = { _ in nil }
    /// Puts around each row what the hosted row needs from the chat's
    /// environment, which doesn't reach into a hosting view.
    var wrap: (String, AnyView) -> AnyView = { $1 }
    /// The chat moved under the pointer.
    var scrolled: () -> Void = {}
    /// The rows are laid out at a new width, so their guesses are worth
    /// working out again.
    var widthChanged: (CGFloat) -> Void = { _ in }

    /// Within this of its end, the chat counts as at its end.
    static let endSlack: CGFloat = 40
    /// Parked hosts kept to be used again; past these, they're let go.
    private static let parkedLimit = 40

    private(set) var rows = ChatRowLayout()
    /// The width the rows were last laid out at.
    private(set) var measuredWidth: CGFloat = 0
    private var versions: [String: UInt32] = [:]
    private var hosts: [String: RowHost] = [:]
    private var parked: [RowHost] = []
    private let footerHost = RowHost()
    private var footerMeasured = false
    private var hasFooter = false
    /// What was in view before the changes since the last layout.
    private var pendingAnchor: ChatRowLayout.Anchor?
    /// A row to go to once it's among the rows.
    private var pendingScroll: (id: String, placement: ChatScrollPlacement, animated: Bool)?
    /// The rows hosted and placed by the last layout.
    private var placed: Range<Int> = 0..<0
    private var arranging = false
    private var insetTop: CGFloat = 0
    private var insetBottom: CGFloat = 0

    override var isFlipped: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        footerHost.isHidden = true
        addSubview(footerHost)
    }

    required init?(coder: NSCoder) { nil }

    /// Whether the end of the chat is in view, or nearly.
    var isAtEnd: Bool { aboveEnd <= Self.endSlack }

    /// How far above its end the chat is scrolled.
    var aboveEnd: CGFloat { max(0, rows.maxOrigin - visibleTop) }

    // MARK: - Changes

    /// Takes the chat's rows as they are now: rows added at either end or
    /// replaced in the middle, and rows whose version moved on measured
    /// again. Whether the rows' ids changed.
    @discardableResult
    func setRows(_ keys: [ChatWindowKey], redraw: Bool, rescaled: Bool) -> Bool {
        willChange()
        let old = rows.ids
        let new = keys.map(\.id)
        let changedIDs = old != new
        if changedIDs { splice(old: old, keys: keys) }
        if rescaled { rows.apply(.unmeasureAll) }
        var stale = Set<String>()
        for key in keys {
            if let version = versions[key.id], version != key.version { stale.insert(key.id) }
        }
        if changedIDs || !stale.isEmpty {
            versions = Dictionary(keys.map { ($0.id, $0.version) }) { _, last in last }
        }
        if !stale.isEmpty { rows.apply(.unmeasure(stale)) }
        refreshHosted(force: redraw || rescaled)
        if redraw || rescaled { footerMeasured = false }
        if let pending = pendingScroll, rows.contains(pending.id) {
            scroll(to: pending.id, placement: pending.placement, animated: pending.animated)
        }
        needsLayout = true
        return changedIDs
    }

    /// Shows `footer` under the last row; drawn again only when `redraw`.
    func setFooter(_ footer: AnyView?, redraw: Bool) {
        guard let footer else {
            if hasFooter {
                willChange()
                hasFooter = false
                footerHost.isHidden = true
                rows.apply(.footer(0))
                needsLayout = true
            }
            return
        }
        guard !hasFooter || redraw else { return }
        willChange()
        hasFooter = true
        footerHost.isHidden = false
        footerHost.show(
            ChatWindowRow(view: reporting(footer, id: nil), value: Footer()), as: Self.footerID, force: true)
        footerMeasured = false
        needsLayout = true
    }

    /// Leaves room over and under the rows, for what's drawn over the chat.
    func setInsets(top: CGFloat, bottom: CGFloat) {
        guard top != insetTop || bottom != insetBottom else { return }
        willChange()
        insetTop = top
        insetBottom = bottom
        enclosingScrollView?.contentInsets = NSEdgeInsets(top: top, left: 0, bottom: bottom, right: 0)
        needsLayout = true
    }

    /// Guessed heights, which only rows not yet measured take.
    func takeEstimates(_ heights: [String: CGFloat]) {
        guard !heights.isEmpty else { return }
        willChange()
        rows.apply(.estimates(heights))
        needsLayout = true
    }

    /// The row `id` is about to change its own height: it's measured again.
    func invalidate(_ id: String) {
        guard rows.contains(id) else { return }
        willChange()
        rows.apply(.unmeasure([id]))
        needsLayout = true
    }

    /// Takes the chat to its end and keeps it there.
    func scrollToEnd() {
        following = true
        pendingScroll = nil
        pendingAnchor = .end
        needsLayout = true
    }

    /// Takes the chat to the row `id`, measuring it and a screen around it
    /// first so it lands where it's asked to. Until the row is among the
    /// chat's, it waits.
    func scroll(to id: String, placement: ChatScrollPlacement, animated: Bool) {
        guard let row = rows.position(of: id), let scroll = enclosingScrollView, rows.viewport > 0 else {
            pendingScroll = (id, placement, animated)
            needsLayout = true
            return
        }
        pendingScroll = nil
        following = false
        measure(around: row)
        let top = rows.top(at: row)
        let wanted =
            switch placement {
            case .top: top - ChatRowLayout.spacing
            case .center: top + rows.heights[row] / 2 - rows.viewport / 2
            }
        let target = min(max(0, wanted), rows.maxOrigin)
        let clip = scroll.contentView
        if animated {
            pendingAnchor = nil
            NSAnimationContext.beginGrouping()
            NSAnimationContext.current.duration = 0.25
            clip.animator().setBoundsOrigin(NSPoint(x: 0, y: target - insetTop))
            scroll.reflectScrolledClipView(clip)
            NSAnimationContext.endGrouping()
        } else {
            pendingAnchor = .row(id: id, offset: target - top, fallback: target)
        }
        needsLayout = true
    }

    // MARK: - Scrolling by hand

    func clipMoved() {
        guard !arranging else { return }
        scrolled()
        // A step that brings no new row in does nothing more.
        let range = rows.range(in: window(at: visibleTop))
        if range.lowerBound < placed.lowerBound || range.upperBound > placed.upperBound {
            needsLayout = true
        }
    }

    /// The person's hand is on the scrolling: rows measured as it moves
    /// don't take the chat back to its end under it.
    func handStarted() {
        following = false
    }

    /// The scrolling stopped, coasting included: where it stopped says
    /// whether the chat keeps to its end.
    func handEnded() {
        following = isAtEnd
    }

    // MARK: - Layout

    override func layout() {
        super.layout()
        arrange()
    }

    /// Measures the rows that need it around what's in view, places them,
    /// and scrolls to keep what was in view where it was.
    private func arrange() {
        guard !arranging, let scroll = enclosingScrollView else { return }
        arranging = true
        defer { arranging = false }
        let clip = scroll.contentView
        let width = clip.bounds.width
        let anchor = pendingAnchor ?? rows.anchor(visible: visible, following: following)
        pendingAnchor = nil
        if abs(width - measuredWidth) > 0.5 {
            measuredWidth = width
            rows.apply(.unmeasureAll)
            footerMeasured = false
            widthChanged(width)
        }
        rows.apply(.viewport(max(0, clip.bounds.height - insetTop - insetBottom)))
        if hasFooter, !footerMeasured {
            rows.apply(.footer(footerHost.measure(width: width)))
            footerMeasured = true
        }
        var origin = rows.origin(for: anchor)
        // Rows measured above move the rows in view: a few rounds settle it.
        for _ in 0..<4 {
            var measuredAny = false
            for row in rows.range(in: window(at: origin)) where !rows.measured[row] {
                let id = rows.ids[row]
                guard let host = host(for: id) else { continue }
                rows.setHeight(host.measure(width: width), of: id, measured: true)
                measuredAny = true
            }
            origin = rows.origin(for: anchor)
            if !measuredAny { break }
        }
        place(rows.range(in: window(at: origin)), width: width)
        let size = NSSize(width: width, height: rows.documentHeight)
        if frame.size != size { setFrameSize(size) }
        let clipOrigin = NSPoint(x: 0, y: origin - insetTop)
        if clip.bounds.origin != clipOrigin {
            clip.setBoundsOrigin(clipOrigin)
            scroll.reflectScrolledClipView(clip)
        }
        // A jump asked for before the pane had a size goes now it has one.
        if let pending = pendingScroll, rows.contains(pending.id), rows.viewport > 0 {
            self.scroll(to: pending.id, placement: pending.placement, animated: pending.animated)
        }
    }

    // MARK: - Private

    private struct Footer: Equatable {}

    private static let footerID = "chat.footer"

    /// The top of what's in view below the inset, in the rows' terms.
    private var visibleTop: CGFloat {
        (enclosingScrollView?.contentView.bounds.minY ?? 0) + insetTop
    }

    private var visible: CGRect {
        CGRect(x: 0, y: visibleTop, width: measuredWidth, height: rows.viewport)
    }

    /// What's in view at `origin` and a screen above and below it.
    private func window(at origin: CGFloat) -> CGRect {
        let screen = max(rows.viewport, 1)
        return CGRect(x: 0, y: origin - screen, width: measuredWidth, height: screen * 3)
    }

    /// Keeps what's in view now, for the changes until the next layout.
    private func willChange() {
        guard pendingAnchor == nil, enclosingScrollView != nil else { return }
        pendingAnchor = rows.anchor(visible: visible, following: following)
    }

    private func splice(old: [String], keys: [ChatWindowKey]) {
        let new = keys.map(\.id)
        let shorter = min(old.count, new.count)
        var prefix = 0
        while prefix < shorter, old[prefix] == new[prefix] { prefix += 1 }
        var suffix = 0
        while suffix < shorter - prefix, old[old.count - 1 - suffix] == new[new.count - 1 - suffix] { suffix += 1 }
        let removed = prefix..<(old.count - suffix)
        // A row that moved keeps its height, measured or not.
        var kept: [String: ChatRowLayout.Row] = [:]
        for row in removed {
            kept[old[row]] = ChatRowLayout.Row(id: old[row], height: rows.heights[row], measured: rows.measured[row])
        }
        let inserted = keys[prefix..<(new.count - suffix)].map { key in
            kept[key.id] ?? ChatRowLayout.Row(id: key.id, height: key.guess)
        }
        rows.apply(.splice(removed, inserted))
        let present = Set(new)
        for (id, host) in hosts where !present.contains(id) { park(host, id: id) }
    }

    /// Builds each hosted row again; a row whose view changed is measured
    /// again. Only the hosted rows: the rest are built when they're hosted.
    private func refreshHosted(force: Bool) {
        var resized = Set<String>()
        for (id, host) in hosts {
            guard let row = rowBuilder(id) else { continue }
            if host.show(wrapped(row, id: id), as: id, force: force) { resized.insert(id) }
        }
        if !resized.isEmpty { rows.apply(.unmeasure(resized)) }
    }

    private func wrapped(_ row: ChatWindowRow, id: String) -> ChatWindowRow {
        ChatWindowRow(view: reporting(wrap(id, row.view), id: id), value: row.value)
    }

    /// `view` telling this when its height changes by itself, as a diagram
    /// drawn late or a thumbnail loaded; nil `id` for the footer.
    private func reporting(_ view: AnyView, id: String?) -> AnyView {
        AnyView(
            RowSizeReporter(content: view) { [weak self] height in
                self?.reported(height, of: id)
            })
    }

    private func reported(_ height: CGFloat, of id: String?) {
        let height = height.rounded(.up)
        guard let id else {
            guard hasFooter, footerMeasured, abs(height - rows.footer) > 0.5 else { return }
            willChange()
            rows.apply(.footer(height))
            needsLayout = true
            return
        }
        guard let row = rows.position(of: id), rows.measured[row], abs(height - rows.heights[row]) > 0.5,
            let host = hosts[id], abs(host.frame.width - measuredWidth) <= 0.5
        else { return }
        willChange()
        rows.setHeight(height, of: id, measured: true)
        needsLayout = true
    }

    /// The host showing the row `id`, hosting it if it isn't: in a parked
    /// host that last showed it if there is one, so it needn't be drawn
    /// again.
    private func host(for id: String) -> RowHost? {
        if let host = hosts[id] { return host }
        guard let row = rowBuilder(id) else { return nil }
        let host: RowHost
        if let index = parked.firstIndex(where: { $0.id == id }) ?? parked.indices.last {
            host = parked.remove(at: index)
        } else {
            host = RowHost()
            addSubview(host)
        }
        if host.show(wrapped(row, id: id), as: id, force: false) { rows.apply(.unmeasure([id])) }
        hosts[id] = host
        return host
    }

    private func park(_ host: RowHost, id: String) {
        hosts[id] = nil
        host.isHidden = true
        if parked.count < Self.parkedLimit {
            parked.append(host)
        } else {
            host.removeFromSuperview()
        }
    }

    /// Measures the row at `row` and a screen of rows above and below it.
    private func measure(around row: Int) {
        let width = measuredWidth
        guard width > 0 else { return }
        func measure(_ index: Int) -> CGFloat {
            let id = rows.ids[index]
            if !rows.measured[index], let host = host(for: id) {
                rows.setHeight(host.measure(width: width), of: id, measured: true)
            }
            return rows.heights[index] + ChatRowLayout.spacing
        }
        _ = measure(row)
        var above: CGFloat = 0
        var index = row - 1
        while index >= 0, above < rows.viewport {
            above += measure(index)
            index -= 1
        }
        var below: CGFloat = 0
        index = row + 1
        while index < rows.count, below < rows.viewport {
            below += measure(index)
            index += 1
        }
    }

    private func place(_ range: Range<Int>, width: CGFloat) {
        var shown = Set<String>()
        for row in range {
            let id = rows.ids[row]
            guard let host = host(for: id) else { continue }
            shown.insert(id)
            let frame = NSRect(x: 0, y: rows.top(at: row), width: width, height: rows.heights[row])
            if host.frame != frame { host.frame = frame }
            host.isHidden = false
        }
        for (id, host) in hosts where !shown.contains(id) { park(host, id: id) }
        if hasFooter {
            let frame = NSRect(x: 0, y: rows.footerTop, width: width, height: rows.footer)
            if footerHost.frame != frame { footerHost.frame = frame }
        }
        placed = range
    }
}

/// One row, hosted. Measured by its hosting controller at a width; placed
/// at the height it measured.
@MainActor
final class RowHost: NSView {
    /// The row it shows, or last showed if parked.
    private(set) var id: String?
    private var value: (any Equatable)?
    private let controller = NSHostingController(rootView: AnyView(EmptyView()))

    /// A row's height is held to this, should one measure without end.
    private static let tallest: CGFloat = 200_000

    override var isFlipped: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        controller.sizingOptions = []
        let hosted = controller.view
        hosted.frame = bounds
        hosted.autoresizingMask = [.width, .height]
        addSubview(hosted)
    }

    required init?(coder: NSCoder) { nil }

    /// Shows `row` as the row `id`, unless it shows it already and isn't
    /// `force`d. Whether it was drawn anew, and so must be measured again.
    @discardableResult
    func show(_ row: ChatWindowRow, as id: String, force: Bool) -> Bool {
        if !force, self.id == id, row.drawsSame(as: value) { return false }
        self.id = id
        value = row.value
        controller.rootView = row.view
        return true
    }

    /// How tall the row is drawn `width` wide.
    func measure(width: CGFloat) -> CGFloat {
        let size = controller.sizeThatFits(in: CGSize(width: width, height: .greatestFiniteMagnitude))
        guard size.height.isFinite else { return 0 }
        return min(max(0, size.height.rounded(.up)), Self.tallest)
    }
}

/// A hosted row at its own height, telling the chat when that height
/// changes after it was measured. Not stretched to the height it's
/// offered: it's measured with no end to that.
private struct RowSizeReporter: View {
    let content: AnyView
    let reported: @MainActor (CGFloat) -> Void

    var body: some View {
        content
            .fixedSize(horizontal: false, vertical: true)
            .onGeometryChange(for: CGFloat.self, of: \.size.height) { reported($0) }
            .frame(maxWidth: .infinity, alignment: .topLeading)
    }
}
