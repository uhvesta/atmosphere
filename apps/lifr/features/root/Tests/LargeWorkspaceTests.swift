import ComposableArchitecture
import Foundation
import LifrCore
import LifrRoot
import LifrStoreClient
import XCTest

/// How long showing a change takes with 10,000 tasks: every screen
/// recomputes its list, so one change must still show well within a frame
/// budget of a few.
@MainActor
final class LargeWorkspaceTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func workspace() -> (Workspace, LifrTask?) {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        _ = records.inbox()
        let keys = ["ONE", "TWO", "THREE", "FOUR", "FIVE", "SIX", "SEVEN", "EIGHT", "NINE", "TEN"]
        let avenues = keys.map { records.avenue($0, $0.capitalized) }
        let projects = avenues.flatMap { avenue in (0..<5).map { records.project("Project \($0)", in: avenue) } }
        var first: LifrTask?
        for index in 0..<10_000 {
            let avenue = avenues[index % avenues.count]
            let project = index % 3 == 0 ? projects[index % projects.count] : nil
            let inAvenue = project.map { $0.avenueID == avenue.id } ?? true
            let task = records.task(
                "Task \(index)", in: avenue, project: inAvenue ? project : nil,
                status: index % 4 == 0 ? .done : .todo, priority: index % 7 == 0 ? .high : .none,
                due: index % 5 == 0 ? nil : index % 60 - 20)
            if first == nil, task.status == .todo, task.dueDate != nil { first = task }
        }
        return (records.workspace, first)
    }

    func testAChangeShowsQuicklyWithTenThousandTasks() throws {
        let (workspace, picked) = workspace()
        let task = try XCTUnwrap(picked)
        XCTAssertGreaterThanOrEqual(workspace.tasks.count, 10_000)
        let reducer = RootFeature()
        var state = RootFeature.State(platform: .mac)
        try withDependencies {
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = .incrementing
        } operation: {
            let loaded = ContinuousClock().measure {
                _ = reducer.reduce(into: &state, action: .loaded(.success(Snapshot(workspace, number: 1))))
            }
            var changed = workspace
            _ = try changed.setStatus(
                .done, of: task.id, in: Context(actor: .user(device: "Mac", surface: .mac), now: now))
            let shown = ContinuousClock().measure {
                _ = reducer.reduce(into: &state, action: .updated(Update(workspace: changed, number: 2)))
            }
            print("10,000 tasks: loaded in \(loaded), a change shown in \(shown)")
            XCTAssertEqual(state.workspace?.tasks[task.id]?.status, .done)
            // Generous for a debug build on a loaded machine; a regression
            // that recomputes per row goes far past it.
            XCTAssertLessThan(shown, .seconds(1))
        }
    }
}
