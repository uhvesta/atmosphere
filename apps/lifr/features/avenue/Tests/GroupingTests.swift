import ComposableArchitecture
import Foundation
import LifrAvenue
import LifrCore
import LifrStoreClient
import XCTest

@MainActor
final class GroupingTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func sections(
        _ grouping: Grouping, _ build: (inout RecordsBuilder, Avenue) -> Void
    ) -> [(String, [String])] {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        build(&records, home)
        let state = AvenueFeature.State(
            scope: .avenue(home.id), workspace: records.workspace, grouping: grouping, now: now, timeZone: .gmt)
        return state.list.sections.map { ($0.title, $0.rows.map(\.title)) }
    }

    func testByPriorityPutsTheMostUrgentFirstAndLeavesOutClosedTasks() {
        let result = sections(.priority) { records, home in
            records.task("Fix the tap", in: home, priority: .urgent)
            records.task("Sweep", in: home)
            records.task("Paint", in: home, priority: .low)
            records.task("Mowed", in: home, status: .done, priority: .high)
        }
        XCTAssertEqual(result.map(\.0), ["Urgent", "Low", "No Priority"])
        XCTAssertEqual(result.map(\.1), [["Fix the tap"], ["Paint"], ["Sweep"]])
    }

    func testByDueDateSplitsOverdueTodayThisWeekLaterAndNone() {
        let result = sections(.due) { records, home in
            records.task("Late", in: home, due: -1)
            records.task("Now", in: home, due: 0)
            records.task("Soon", in: home, due: 7)
            records.task("Far", in: home, due: 8)
            records.task("Whenever", in: home)
        }
        XCTAssertEqual(result.map(\.1), [["Late"], ["Now"], ["Soon"], ["Far"], ["Whenever"]])
    }

    func testByLabelPutsATaskUnderItsFirstLabelOnly() {
        let result = sections(.label) { records, home in
            let phone = records.label("Phone")
            let errand = records.label("errand")
            records.task("Call the plumber", in: home, labels: [phone])
            records.task("Post and call", in: home, labels: [phone, errand])
            records.task("Tidy", in: home)
        }
        XCTAssertEqual(result.map(\.0), ["#errand", "#Phone", "No Label"])
        XCTAssertEqual(result.map(\.1), [["Post and call"], ["Call the plumber"], ["Tidy"]])
    }

    func testNoneShowsEveryOpenTaskTogetherAndLaterOnesApart() {
        let result = sections(.none) { records, home in
            records.task("Sweep", in: home)
            records.task("Fix the tap", in: home, priority: .urgent, due: 2)
            records.task("Mowed", in: home, status: .done)
            records.task("Bulbs", in: home, starts: 3)
        }
        XCTAssertEqual(result.map(\.1).map { Set($0) }, [["Sweep", "Fix the tap"], ["Bulbs"]])
    }

    func testAProjectCantBeGroupedByProject() {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        let shed = records.project("Shed", in: home)
        let workspace = records.workspace
        let avenue = AvenueFeature.State(scope: .avenue(home.id), workspace: workspace, now: now, timeZone: .gmt)
        let project = AvenueFeature.State(scope: .project(shed.id), workspace: workspace, now: now, timeZone: .gmt)
        XCTAssertTrue(avenue.groupings.contains(.project))
        XCTAssertFalse(project.groupings.contains(.project))
    }

    func testTheGroupingIsRememberedForEachAvenue() {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        let work = records.avenue("WORK", "Work")
        let workspace = records.workspace
        var first = AvenueFeature.State(scope: .avenue(home.id), workspace: workspace, now: now, timeZone: .gmt)
        first.$grouping.withLock { $0 = .priority }

        let again = AvenueFeature.State(scope: .avenue(home.id), workspace: workspace, now: now, timeZone: .gmt)
        let other = AvenueFeature.State(scope: .avenue(work.id), workspace: workspace, now: now, timeZone: .gmt)
        XCTAssertEqual(again.grouping, .priority)
        XCTAssertEqual(other.grouping, .status)
    }
}
