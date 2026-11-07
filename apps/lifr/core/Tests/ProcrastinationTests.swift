import Foundation
import LifrCore
import TestingKit

func procrastinationTests() -> [TestCase] {
    [
        TestCase("given In Progress with no activity for 9 days, then it's Procrastinated for 9 days") {
            let fixture = try Fixture()
            let task = try fixture.task("Stretch", status: .inProgress)
            fixture.advance(days: 9.5)
            let settings = fixture.workspace.settings
            try expectEqual(fixture.get(task).procrastination(now: fixture.now, settings: settings), .inactive(days: 9))
            try expectEqual(fixture.get(task).effectiveStatus(now: fixture.now, settings: settings), .procrastinated)
        },
        TestCase("given In Progress for exactly the threshold, then it isn't Procrastinated yet") {
            let fixture = try Fixture()
            let task = try fixture.task("Stretch", status: .inProgress)
            fixture.advance(days: 7)
            try expectEqual(
                fixture.get(task).procrastination(now: fixture.now, settings: fixture.workspace.settings), nil)
        },
        TestCase("given To Do with no activity for a month, then it isn't Procrastinated") {
            let fixture = try Fixture()
            let task = try fixture.task("Stretch")
            fixture.advance(days: 30)
            try expectEqual(
                fixture.get(task).effectiveStatus(now: fixture.now, settings: fixture.workspace.settings), .todo)
        },
        TestCase("given a 3-day threshold in Settings, then 4 idle days are Procrastinated") {
            let fixture = try Fixture()
            fixture.workspace.settings.procrastinatedAfterDays = 3
            let task = try fixture.task("Stretch", status: .inProgress)
            fixture.advance(days: 4)
            try expectEqual(
                fixture.get(task).procrastination(now: fixture.now, settings: fixture.workspace.settings),
                .inactive(days: 4))
        },
        TestCase("given Procrastinated set by hand, then it says so and stays without activity rules") {
            let fixture = try Fixture()
            let task = try fixture.task("Stretch")
            _ = try fixture.workspace.setStatus(.procrastinated, of: task.id, in: fixture.context)
            try expectEqual(
                fixture.get(task).procrastination(now: fixture.now, settings: fixture.workspace.settings), .setByHand)
        },
        TestCase(
            "given an idle In Progress task, when a comment, edit or Sub task change happens, then it's no longer "
                + "Procrastinated"
        ) {
            let fixture = try Fixture()
            let settings = fixture.workspace.settings
            let commented = try fixture.task("Commented", status: .inProgress)
            let edited = try fixture.task("Edited", status: .inProgress)
            let parent = try fixture.task("Parent", status: .inProgress)
            let subtask = try fixture.task("Sub task", parent: parent)
            fixture.advance(days: 10)
            _ = try fixture.workspace.addComment("Still on it", to: commented.id, in: fixture.context)
            _ = try fixture.workspace.edit(edited.id, .init(details: "Halfway"), in: fixture.context)
            _ = try fixture.workspace.setStatus(.inProgress, of: subtask.id, in: fixture.context)
            for task in [commented, edited, parent] {
                try expectEqual(fixture.get(task).effectiveStatus(now: fixture.now, settings: settings), .inProgress)
            }
        },
    ]
}
