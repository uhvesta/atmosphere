import Foundation
import LifrCore
import LifrLists
import LifrStoreClient
import LifrTaskList
import XCTest

final class ListsTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private let day: TimeInterval = 24 * 60 * 60

    private func titles(_ query: TaskQuery, _ workspace: Workspace, at time: Date) -> [String: [String]] {
        Dictionary(
            uniqueKeysWithValues: query.sections(workspace, time, .gmt)
                .filter { !$0.rows.isEmpty }
                .map { ($0.title, $0.rows.map(\.title)) })
    }

    func testATaskStartingTomorrowIsLaterUntilTomorrowThenAnytime() {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        records.task("Paint the fence", in: home, starts: 1)
        records.task("Sort the garage", in: home)

        XCTAssertEqual(titles(.anytime, records.workspace, at: now), ["Home": ["Sort the garage"]])
        XCTAssertEqual(
            titles(.later, records.workspace, at: now).values.flatMap { $0 }, ["Paint the fence"])

        let tomorrow = now + day
        XCTAssertEqual(
            titles(.anytime, records.workspace, at: tomorrow).values.flatMap { $0 }.sorted(),
            ["Paint the fence", "Sort the garage"])
        XCTAssertEqual(titles(.later, records.workspace, at: tomorrow), [:])
    }

    func testAnytimeLeavesOutDatedClosedAndSubTasks() {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        let parent = records.task("Move house", in: home)
        records.task("Pack books", in: home, parent: parent)
        records.task("Pay rent", in: home, due: 3)
        records.task("Old chore", in: home, status: .done)

        XCTAssertEqual(titles(.anytime, records.workspace, at: now), ["Home": ["Move house"]])
    }

    func testTheLogbookPutsTheLatestClosedFirstInDaySections() {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        records.task("Still open", in: home)
        records.task("Done today", in: home, status: .done)
        let logbook = TaskQuery.logbook.sections(records.workspace, now + 3 * day, .gmt)

        XCTAssertEqual(logbook.map(\.id), ["week"])
        XCTAssertEqual(logbook.first?.rows.map(\.title), ["Done today"])

        let later = TaskQuery.logbook.sections(records.workspace, now + 40 * day, .gmt)
        XCTAssertEqual(later.map(\.id), ["2026-11-01"])
    }
}
