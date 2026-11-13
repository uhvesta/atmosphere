import HumataReview
import XCTest

final class MarkdownComparisonTests: XCTestCase {
    func testAnEditedLineAnAddedOneAndARemovalAreMarkedWhereTheyAre() {
        let old = ["# Plan", "", "One.", "Two.", "Three.", "", "End."]
        XCTAssertEqual(
            MarkdownComparison.changedLines(from: old, to: ["# Plan", "", "One.", "2.", "Three.", "", "End."]), [4])
        XCTAssertEqual(
            MarkdownComparison.changedLines(
                from: old, to: ["# Plan", "", "One.", "Two.", "More.", "Three.", "", "End."]), [5])
        // Where a line went, the line now in its place is marked.
        XCTAssertEqual(
            MarkdownComparison.changedLines(from: old, to: ["# Plan", "", "One.", "Three.", "", "End."]), [4])
        XCTAssertEqual(MarkdownComparison.changedLines(from: old, to: old), [])
        XCTAssertEqual(MarkdownComparison.changedLines(from: old, to: []), [])
        XCTAssertEqual(MarkdownComparison.changedLines(from: old, to: Array(old.dropLast())), [6])
    }

    /// A rewrite this long would take a line diff minutes, so the test
    /// ending is what shows it isn't diffed.
    func testALongRewrittenMiddleIsMarkedWholeBetweenTheLinesKept() {
        let head = ["# Plan", ""]
        let tail = ["", "End."]
        let old = head + (0..<30_000).map { "old \($0)" } + tail
        let new = head + (0..<25_000).map { "new \($0)" } + tail

        let changed = MarkdownComparison.changedLines(from: old, to: new)

        XCTAssertEqual(changed, Set(3...25_002))
    }

    func testALongMiddleRemovedMarksTheLineNowInItsPlace() {
        let old = ["# Plan"] + (0..<30_000).map { "old \($0)" } + ["End."]

        XCTAssertEqual(MarkdownComparison.changedLines(from: old, to: ["# Plan", "End."]), [2])
    }
}
