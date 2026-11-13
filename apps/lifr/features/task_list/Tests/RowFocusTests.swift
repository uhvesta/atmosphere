import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskList
import XCTest

/// Where VoiceOver goes when the row it's on leaves the list.
final class RowFocusTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    /// Two sections, "a" with tasks 0 to 2 and "b" with 3 and 4, then the
    /// same sections keeping only `kept`.
    private func lists(keeping kept: Set<Int>) -> (before: [TaskSection], after: [TaskSection], ids: [UUID]) {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let avenue = records.avenue("HOME", "Home")
        let tasks = (0..<5).map { records.task("Task \($0)", in: avenue) }
        let workspace = records.workspace
        func sections(_ indices: Set<Int>) -> [TaskSection] {
            [(0..<3), (3..<5)].enumerated().map { number, range in
                let picked = range.filter(indices.contains).map { tasks[$0] }
                return TaskSection(
                    id: "\(number)", title: "\(number)", rows: workspace.rows(picked, now: now, timeZone: .gmt))
            }
        }
        return (sections(Set(0..<5)), sections(kept), tasks.map(\.id))
    }

    func testFocusMovesToTheNextRowEvenInTheNextSection() {
        let (before, after, ids) = lists(keeping: [0, 1, 3, 4])
        XCTAssertEqual(after.focus(after: ids[2], leaving: before), ids[3])
        let (_, middle, _) = lists(keeping: [0, 2, 3, 4])
        XCTAssertEqual(middle.focus(after: ids[1], leaving: before), ids[2])
    }

    func testTheLastRowGivesFocusToTheOneBefore() {
        let (before, after, ids) = lists(keeping: [0, 1, 2])
        XCTAssertEqual(after.focus(after: ids[4], leaving: before), ids[2])
    }

    func testARowStillShownOrAnEmptyListMovesNothing() {
        let (before, after, ids) = lists(keeping: [0, 1, 3, 4])
        XCTAssertNil(after.focus(after: ids[0], leaving: before))
        let (_, empty, _) = lists(keeping: [])
        XCTAssertNil(empty.focus(after: ids[0], leaving: before))
    }
}
