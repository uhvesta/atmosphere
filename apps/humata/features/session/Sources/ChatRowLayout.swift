import CoreGraphics

/// Where a chat's rows sit, worked out from their heights alone: each row's
/// top, the rows in a stretch of the chat, and where the chat must be
/// scrolled to after a change so what was in view stays where it was.
///
/// A value, and no view's: the chat's scroll view keeps one and asks it
/// where to put things, so prepending rows, a row measured above what's in
/// view and a new width all keep the same row under the same point.
struct ChatRowLayout: Sendable {
    /// One row: its id and the height it has, measured or guessed.
    struct Row: Equatable, Sendable {
        let id: String
        var height: CGFloat
        /// The height is the row's own, laid out; otherwise it's a guess.
        var measured: Bool

        init(id: String, height: CGFloat, measured: Bool = false) {
            self.id = id
            self.height = height
            self.measured = measured
        }
    }

    /// What's in view, taken before a change so it can be kept after it.
    enum Anchor: Equatable, Sendable {
        /// The chat's end: it keeps to its end as it grows.
        case end
        /// The row `id` stays where it was, `offset` points of it above the
        /// top of what's in view. `fallback` is where the view was, kept
        /// if the row goes.
        case row(id: String, offset: CGFloat, fallback: CGFloat)
    }

    /// Something that changes the rows or the room they have.
    enum Change: Sendable {
        /// Rows after the last.
        case append([Row])
        /// Rows before the first, as earlier messages shown.
        case prepend([Row])
        /// The rows at `range` replaced by these, in order.
        case splice(Range<Int>, [Row])
        /// Every row replaced.
        case reset([Row])
        /// The rows with these ids taken out.
        case remove(Set<String>)
        /// Heights the rows were measured at.
        case heights([String: CGFloat])
        /// Guessed heights, which only rows not yet measured take.
        case estimates([String: CGFloat])
        /// The rows with these ids must be measured again, as what they
        /// show changed: until they are, they keep the height they had.
        case unmeasure(Set<String>)
        /// Every row must be measured again, as at a new width or text
        /// scale.
        case unmeasureAll
        /// The height of what's under the last row, the turn under way.
        case footer(CGFloat)
        /// The height of the pane the chat shows in.
        case viewport(CGFloat)
    }

    /// Between two rows, and between the last row and the footer.
    static let spacing: CGFloat = 14
    /// Around all the rows, top and bottom.
    static let padding: CGFloat = 16

    private(set) var ids: [String] = []
    private(set) var heights: [CGFloat] = []
    private(set) var measured: [Bool] = []
    private(set) var footer: CGFloat = 0
    private(set) var viewport: CGFloat = 0
    private var index: [String: Int] = [:]
    /// `prefix[row]` is the height of the rows before it, each with its
    /// spacing: one more than there are rows.
    private var prefix: [CGFloat] = [0]

    init(rows: [Row] = []) {
        reset(rows)
    }

    var count: Int { ids.count }

    var isEmpty: Bool { ids.isEmpty }

    func contains(_ id: String) -> Bool { index[id] != nil }

    func position(of id: String) -> Int? { index[id] }

    func isMeasured(_ id: String) -> Bool {
        guard let row = index[id] else { return false }
        return measured[row]
    }

    /// The rows and footer with the padding around them.
    var totalHeight: CGFloat {
        let footerPart = footer > 0 ? (ids.isEmpty ? 0 : Self.spacing) + footer : 0
        return Self.padding * 2 + rowsHeight + footerPart
    }

    /// How tall the scrolled view is: never shorter than the pane, so a
    /// short chat sits at its bottom.
    var documentHeight: CGFloat { max(totalHeight, viewport) }

    /// The furthest the chat scrolls down.
    var maxOrigin: CGFloat { max(0, documentHeight - viewport) }

    /// The top of the row `id`, in the scrolled view; nil if there's none.
    func top(of id: String) -> CGFloat? {
        index[id].map(top(at:))
    }

    /// The top of the row at `row`, which must be one of the rows.
    func top(at row: Int) -> CGFloat {
        lead + Self.padding + prefix[row]
    }

    /// Where the footer starts.
    var footerTop: CGFloat {
        lead + Self.padding + rowsHeight + (ids.isEmpty || footer <= 0 ? 0 : Self.spacing)
    }

    /// The rows any part of which is within `rect`'s height, in the
    /// scrolled view.
    func range(in rect: CGRect) -> Range<Int> {
        guard !ids.isEmpty else { return 0..<0 }
        let start = rect.minY - lead - Self.padding
        let end = rect.maxY - lead - Self.padding
        // The first row whose bottom is below the start.
        let lower = firstRow { prefix[$0] + heights[$0] > start }
        // The first row whose top is at or past the end.
        let upper = firstRow { prefix[$0] >= end }
        return lower..<max(lower, upper)
    }

    /// What `visible` shows, to keep through a change: its end while
    /// `following`, otherwise its first row whose bottom is in view.
    func anchor(visible: CGRect, following: Bool) -> Anchor {
        guard !following, !ids.isEmpty else { return .end }
        let first = firstRow { prefix[$0] + heights[$0] > visible.minY - lead - Self.padding }
        let row = min(first, ids.count - 1)
        return .row(id: ids[row], offset: visible.minY - top(at: row), fallback: visible.minY)
    }

    /// Where to scroll to so `anchor` is where it was.
    func origin(for anchor: Anchor) -> CGFloat {
        switch anchor {
        case .end:
            return maxOrigin
        case .row(let id, let offset, let fallback):
            let wanted = top(of: id).map { $0 + offset } ?? fallback
            return min(max(0, wanted), maxOrigin)
        }
    }

    /// Makes `change`, and gives where to scroll to so `anchor` stays
    /// where it was.
    mutating func apply(_ change: Change, keeping anchor: Anchor) -> CGFloat {
        apply(change)
        return origin(for: anchor)
    }

    /// Makes `change`, leaving where to scroll to the caller.
    mutating func apply(_ change: Change) {
        switch change {
        case .append(let rows):
            splice(ids.count..<ids.count, with: rows)
        case .prepend(let rows):
            splice(0..<0, with: rows)
        case .splice(let range, let rows):
            let lower = min(max(0, range.lowerBound), ids.count)
            let upper = min(max(lower, range.upperBound), ids.count)
            splice(lower..<upper, with: rows)
        case .reset(let rows):
            reset(rows)
        case .remove(let gone):
            let kept = ids.indices.filter { !gone.contains(ids[$0]) }
            guard kept.count < ids.count else { return }
            reset(kept.map { Row(id: ids[$0], height: heights[$0], measured: measured[$0]) })
        case .heights(let measuredHeights):
            setHeights(measuredHeights, measured: true)
        case .estimates(let guessed):
            setHeights(guessed, measured: false)
        case .unmeasure(let rows):
            for row in rows.compactMap({ index[$0] }) { measured[row] = false }
        case .unmeasureAll:
            measured = Array(repeating: false, count: ids.count)
        case .footer(let height):
            footer = max(0, height)
        case .viewport(let height):
            viewport = max(0, height)
        }
    }

    /// Gives the row `id` a height: one measured, or a guess, which a
    /// measured row doesn't take. Whether its height changed.
    @discardableResult
    mutating func setHeight(_ height: CGFloat, of id: String, measured isMeasured: Bool) -> Bool {
        guard let row = index[id] else { return false }
        if !isMeasured, measured[row] { return false }
        measured[row] = isMeasured || measured[row]
        guard heights[row] != height else { return false }
        heights[row] = max(0, height)
        rebuild(from: row)
        return true
    }

    // MARK: - Private

    /// How far below the pane's top the rows start: a chat shorter than
    /// the pane sits at its bottom.
    private var lead: CGFloat { max(0, viewport - totalHeight) }

    private var rowsHeight: CGFloat {
        ids.isEmpty ? 0 : prefix[ids.count] - Self.spacing
    }

    /// The first row for which `isPast` holds, the rows before it all not;
    /// the count of rows if there's none.
    private func firstRow(where isPast: (Int) -> Bool) -> Int {
        var low = 0
        var high = ids.count
        while low < high {
            let middle = (low + high) / 2
            if isPast(middle) {
                high = middle
            } else {
                low = middle + 1
            }
        }
        return low
    }

    private mutating func setHeights(_ given: [String: CGFloat], measured isMeasured: Bool) {
        var first: Int?
        for (id, height) in given {
            guard let row = index[id], isMeasured || !measured[row] else { continue }
            if isMeasured { measured[row] = true }
            guard heights[row] != height else { continue }
            heights[row] = max(0, height)
            first = min(first ?? row, row)
        }
        if let first { rebuild(from: first) }
    }

    private mutating func reset(_ rows: [Row]) {
        ids = rows.map(\.id)
        heights = rows.map { max(0, $0.height) }
        measured = rows.map(\.measured)
        index = [:]
        index.reserveCapacity(rows.count)
        for (row, id) in ids.enumerated() { index[id] = row }
        prefix = [0]
        rebuild(from: 0)
    }

    private mutating func splice(_ range: Range<Int>, with rows: [Row]) {
        guard !range.isEmpty || !rows.isEmpty else { return }
        for id in ids[range] { index[id] = nil }
        ids.replaceSubrange(range, with: rows.map(\.id))
        heights.replaceSubrange(range, with: rows.map { max(0, $0.height) })
        measured.replaceSubrange(range, with: rows.map(\.measured))
        // The rows after the change moved only when their count did.
        let end = rows.count == range.count ? range.lowerBound + rows.count : ids.count
        for row in range.lowerBound..<end { index[ids[row]] = row }
        rebuild(from: range.lowerBound)
    }

    /// Works out the tops again from the row `start` on.
    private mutating func rebuild(from start: Int) {
        let start = min(start, ids.count)
        prefix.removeSubrange((start + 1)...)
        prefix.reserveCapacity(ids.count + 1)
        var top = prefix[start]
        for row in start..<ids.count {
            top += heights[row] + Self.spacing
            prefix.append(top)
        }
    }
}
