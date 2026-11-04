import Foundation
import LifrCore
import LifrTesting

func revertTests() -> [TestCase] {
    [
        TestCase("given a renamed task, when the rename is reverted, then the old title is back as an undo") {
            let fixture = try Fixture()
            let task = try fixture.task("Run")
            let rename = try fixture.workspace.edit(task.id, .init(title: "Run 5 km"), in: fixture.context)
            let change = try fixture.workspace.revert(rename.events[0].id, in: fixture.context)
            try expectEqual(fixture.get(task).title, "Run")
            try expectEqual(change.events.map(\.reason), [.undo])
        },
        TestCase("given a removed due date, when that is reverted, then the date is back") {
            let fixture = try Fixture()
            let task = try fixture.task("Run")
            let day = try CalendarDate(year: 2026, month: 11, day: 5)
            _ = try fixture.workspace.edit(task.id, .init(dueDate: .some(day)), in: fixture.context)
            let removal = try fixture.workspace.edit(task.id, .init(dueDate: .some(nil)), in: fixture.context)
            _ = try fixture.workspace.revert(removal.events[0].id, in: fixture.context)
            try expectEqual(fixture.get(task).dueDate, day)
        },
        TestCase("given a status change, when it's reverted, then the old status is back") {
            let fixture = try Fixture()
            let task = try fixture.task("Run")
            let done = try fixture.workspace.setStatus(.done, of: task.id, in: fixture.context)
            let event = done.events.first { $0.field == .status }!
            _ = try fixture.workspace.revert(event.id, in: fixture.context)
            try expectEqual(fixture.get(task).status, .todo)
        },
        TestCase("given a field changed again since, when the earlier change is reverted, then changedSince") {
            let fixture = try Fixture()
            let task = try fixture.task("Run")
            let first = try fixture.workspace.edit(task.id, .init(title: "Run 5 km"), in: fixture.context)
            _ = try fixture.workspace.edit(task.id, .init(title: "Run 10 km"), in: fixture.context)
            try expectError(.changedSince(event: first.events[0].id)) {
                try fixture.workspace.revert(first.events[0].id, in: fixture.context)
            }
            try expectEqual(fixture.get(task).title, "Run 10 km")
        },
        TestCase("given a task's creation, when it's reverted, then cannotRevert") {
            let fixture = try Fixture()
            let task = try fixture.task("Run")
            let created = fixture.workspace.history.first { $0.taskID == task.id && $0.kind == .created }!
            try expectError(.cannotRevert(event: created.id)) {
                try fixture.workspace.revert(created.id, in: fixture.context)
            }
        },
    ]
}
