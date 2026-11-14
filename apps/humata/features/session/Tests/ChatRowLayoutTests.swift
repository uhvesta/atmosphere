import CoreGraphics
import XCTest

@testable import HumataSession

/// Where a chat's rows sit from their heights, and where the chat is
/// scrolled to after a change so what was in view stays put.
final class ChatRowLayoutTests: XCTestCase {
    private let spacing = ChatRowLayout.spacing
    private let padding = ChatRowLayout.padding

    /// `count` rows named `r0`, `r1` and on, each `height` tall and measured.
    private func rows(_ count: Int, height: CGFloat = 100, from first: Int = 0) -> [ChatRowLayout.Row] {
        (first..<(first + count)).map { ChatRowLayout.Row(id: "r\($0)", height: height, measured: true) }
    }

    private func makeLayout(_ count: Int, height: CGFloat = 100, viewport: CGFloat = 500) -> ChatRowLayout {
        var layout = ChatRowLayout(rows: rows(count, height: height))
        layout.apply(.viewport(viewport))
        return layout
    }

    private func visible(_ layout: ChatRowLayout, at origin: CGFloat) -> CGRect {
        CGRect(x: 0, y: origin, width: 600, height: layout.viewport)
    }

    func testRowsSitOneAfterAnotherAndARectHoldsTheRowsItCrosses() throws {
        var layout = ChatRowLayout(rows: [
            .init(id: "a", height: 50, measured: true),
            .init(id: "b", height: 200, measured: true),
            .init(id: "c", height: 30, measured: true),
        ])
        XCTAssertEqual(layout.top(of: "a"), padding)
        XCTAssertEqual(layout.top(of: "b"), padding + 50 + spacing)
        XCTAssertEqual(layout.top(of: "c"), padding + 250 + spacing * 2)
        XCTAssertNil(layout.top(of: "missing"))
        XCTAssertEqual(layout.totalHeight, padding * 2 + 280 + spacing * 2)

        let bTop = try XCTUnwrap(layout.top(of: "b"))
        XCTAssertEqual(layout.range(in: CGRect(x: 0, y: 0, width: 10, height: padding + 10)), 0..<1)
        XCTAssertEqual(layout.range(in: CGRect(x: 0, y: bTop + 10, width: 10, height: 20)), 1..<2)
        XCTAssertEqual(layout.range(in: CGRect(x: 0, y: 0, width: 10, height: 10_000)), 0..<3)
        // In the gap between two rows: neither.
        XCTAssertEqual(layout.range(in: CGRect(x: 0, y: bTop - spacing + 1, width: 10, height: 2)).count, 0)

        layout.apply(.footer(40))
        XCTAssertEqual(layout.totalHeight, padding * 2 + 280 + spacing * 3 + 40)
        XCTAssertEqual(layout.footerTop, padding + 280 + spacing * 3)
    }

    func testAShortChatSitsAtTheBottomOfThePane() throws {
        let layout = makeLayout(2, height: 100, viewport: 1_000)
        let total = padding * 2 + 200 + spacing
        XCTAssertEqual(layout.documentHeight, 1_000)
        XCTAssertEqual(layout.maxOrigin, 0)
        XCTAssertEqual(layout.top(of: "r0"), 1_000 - total + padding)
        let last = try XCTUnwrap(layout.top(of: "r1"))
        XCTAssertEqual(last + 100 + padding, 1_000)
    }

    func testPrependingRowsKeepsTheAnchoredRowWhereItWasOnScreen() throws {
        var layout = makeLayout(50)
        let origin: CGFloat = 1_234
        let anchor = layout.anchor(visible: visible(layout, at: origin), following: false)
        guard case .row(let id, let offset, _) = anchor else { return XCTFail("anchored to the end") }
        let screenBefore = try XCTUnwrap(layout.top(of: id)) - origin

        let newOrigin = layout.apply(.prepend(rows(30, height: 70, from: 100)), keeping: anchor)
        XCTAssertEqual(layout.count, 80)
        XCTAssertEqual(try XCTUnwrap(layout.top(of: id)) - newOrigin, screenBefore, accuracy: 0.001)
        XCTAssertEqual(newOrigin - (try XCTUnwrap(layout.top(of: id))), offset, accuracy: 0.001)
        XCTAssertEqual(newOrigin, origin + 30 * (70 + spacing), accuracy: 0.001)
    }

    func testAHeightChangeAboveTheViewMovesTheOriginByItAndOneBelowDoesNot() {
        var layout = makeLayout(50)
        let origin: CGFloat = 2_000
        let anchor = layout.anchor(visible: visible(layout, at: origin), following: false)

        layout.setHeight(160, of: "r2", measured: true)
        XCTAssertEqual(layout.origin(for: anchor), origin + 60, accuracy: 0.001)

        layout.setHeight(400, of: "r45", measured: true)
        XCTAssertEqual(layout.origin(for: anchor), origin + 60, accuracy: 0.001)

        let batched = layout.apply(.heights(["r0": 50, "r48": 10]), keeping: anchor)
        XCTAssertEqual(batched, origin + 10, accuracy: 0.001)
    }

    func testFollowingKeepsTheChatAtItsEndAsItGrows() {
        var layout = makeLayout(10)
        let anchor = layout.anchor(visible: visible(layout, at: layout.maxOrigin), following: true)
        XCTAssertEqual(anchor, .end)

        var origin = layout.apply(.append(rows(5, from: 10)), keeping: anchor)
        XCTAssertEqual(origin + layout.viewport, layout.documentHeight)
        origin = layout.apply(.footer(300), keeping: anchor)
        XCTAssertEqual(origin + layout.viewport, layout.documentHeight)
        origin = layout.apply(.viewport(320), keeping: anchor)
        XCTAssertEqual(origin + layout.viewport, layout.documentHeight)
    }

    func testANewWidthKeepsTheAnchoredRowInPlace() throws {
        var layout = makeLayout(40)
        let origin: CGFloat = 1_500
        let anchor = layout.anchor(visible: visible(layout, at: origin), following: false)
        guard case .row(let id, let offset, _) = anchor else { return XCTFail("anchored to the end") }

        layout.apply(.unmeasureAll)
        XCTAssertFalse(layout.isMeasured(id))
        // Narrower, every row is taller.
        let taller = Dictionary(uniqueKeysWithValues: layout.ids.map { ($0, CGFloat(180)) })
        let newOrigin = layout.apply(.heights(taller), keeping: anchor)
        XCTAssertEqual(newOrigin - (try XCTUnwrap(layout.top(of: id))), offset, accuracy: 0.001)
        XCTAssertTrue(layout.isMeasured(id))
    }

    func testEstimatesReachOnlyRowsNotYetMeasured() {
        var layout = ChatRowLayout(rows: [
            .init(id: "seen", height: 100, measured: true),
            .init(id: "unseen", height: 40),
        ])
        layout.apply(.estimates(["seen": 300, "unseen": 300]))
        XCTAssertEqual(layout.heights, [100, 300])
        XCTAssertFalse(layout.setHeight(10, of: "seen", measured: false))
        XCTAssertEqual(layout.heights, [100, 300])
        XCTAssertFalse(layout.isMeasured("unseen"))
    }

    func testRemovingTheAnchoredRowKeepsWhereTheViewWas() {
        var layout = makeLayout(30)
        let origin: CGFloat = 900
        let anchor = layout.anchor(visible: visible(layout, at: origin), following: false)
        guard case .row(let id, _, _) = anchor else { return XCTFail("anchored to the end") }
        let newOrigin = layout.apply(.remove([id]), keeping: anchor)
        XCTAssertEqual(layout.count, 29)
        XCTAssertFalse(layout.contains(id))
        XCTAssertEqual(newOrigin, origin)
    }

    func testSplicingReplacesRowsInOrder() {
        var layout = makeLayout(5)
        let replacing: [ChatRowLayout.Row] = [
            .init(id: "x", height: 10), .init(id: "y", height: 20), .init(id: "z", height: 30),
        ]
        layout.apply(.splice(1..<3, replacing))
        XCTAssertEqual(layout.ids, ["r0", "x", "y", "z", "r3", "r4"])
        XCTAssertEqual(layout.position(of: "r4"), 5)
        XCTAssertNil(layout.position(of: "r1"))
        XCTAssertEqual(layout.top(of: "r3"), layout.top(of: "r0").map { $0 + 100 + 60 + spacing * 4 })
    }
}
