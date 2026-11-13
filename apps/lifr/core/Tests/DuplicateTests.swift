import Foundation
import LifrCore
import TestingKit

func duplicateTests() -> [TestCase] {
    [
        TestCase("given a task, then its copy is open, right after it, with its fields and open Sub tasks") {
            let fixture = try Fixture()
            let parent = try fixture.task("Plan the race", status: .inProgress, project: fixture.marathon)
            let later = try fixture.task("Buy shoes")
            let label = fixture.workspace.createLabel(name: "race", in: fixture.context)
            let due = try CalendarDate(year: 2026, month: 12, day: 1)
            var edit = Workspace.TaskEdit(details: "Pick a route.", priority: .high, labelIDs: [label.id])
            edit.dueDate = .some(due)
            _ = try fixture.workspace.edit(parent.id, edit, in: fixture.context)
            _ = try fixture.workspace.addReminder(.beforeDue(seconds: 3600), to: parent.id, in: fixture.context)
            let open = try fixture.task("Book a hotel", parent: parent)
            _ = try fixture.task("Sign up", status: .done, parent: parent)

            let change = try fixture.workspace.duplicate(task: parent.id, in: fixture.context)

            let created = change.previous.filter { $0.value == nil }.map(\.key)
            let copy = try fixture.workspace.tasks.values
                .first { created.contains($0.id) && $0.parentID == nil }.unwrap("no copy")
            try expectEqual(copy.title, parent.title)
            try expectEqual(copy.status, .todo)
            try expectEqual(copy.details, "Pick a route.")
            try expectEqual(copy.priority, .high)
            try expectEqual(copy.labelIDs, [label.id])
            try expectEqual(copy.dueDate, due)
            try expectEqual(copy.projectID, fixture.marathon.id)
            try expect(copy.code != parent.code)
            try expectEqual(fixture.workspace.subtasks(of: copy.id).map(\.title), [open.title])
            try expectEqual(fixture.workspace.reminders.values.count { $0.taskID == copy.id }, 1)
            let order = fixture.workspace.tasks.values.filter { $0.parentID == nil }
                .sorted(by: LifrTask.siblingOrder).map(\.id)
            try expectEqual(order, [parent.id, copy.id, later.id])
        },
        TestCase("given a duplicate, when it's undone, then the copy and its Sub tasks are gone") {
            let fixture = try Fixture()
            let parent = try fixture.task("Plan the race")
            _ = try fixture.task("Book a hotel", parent: parent)
            let before = Set(fixture.workspace.tasks.keys)

            let change = try fixture.workspace.duplicate(task: parent.id, in: fixture.context)
            _ = fixture.workspace.undo(change, in: fixture.context)

            try expectEqual(Set(fixture.workspace.tasks.values.filter { $0.deletedAt == nil }.map(\.id)), before)
        },
    ]
}
