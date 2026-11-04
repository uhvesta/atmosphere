import Foundation
import LifrCore
import LifrTesting

func statusTests() -> [TestCase] {
    [
        TestCase("given an open Sub task, when the parent moves to Done, then openSubtasks lists it") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent")
            let open = try fixture.task("Open", parent: parent)
            try fixture.task("Closed", status: .done, parent: parent)
            try expectError(.openSubtasks(task: parent.id, open: [open.id])) {
                try fixture.workspace.setStatus(.done, of: parent.id, in: fixture.context)
            }
            try expectEqual(fixture.get(parent).status, .todo)
        },
        TestCase("given open Sub tasks, when the parent moves to Done with cascade, then they close as Won't Do") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent")
            let first = try fixture.task("First", parent: parent)
            let second = try fixture.task("Second", parent: parent)
            let change = try fixture.workspace.setStatus(.done, of: parent.id, cascade: true, in: fixture.context)
            try expectEqual(fixture.get(parent).status, .done)
            try expectEqual(fixture.get(first).status, .wontDo)
            try expectEqual(fixture.get(second).status, .wontDo)
            try expectEqual(change.events.filter { $0.field == .status }.count, 3)
        },
        TestCase("given every Sub task closed, when the parent moves to Done, then closedAt is set") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent")
            let subtask = try fixture.task("Sub task", parent: parent)
            _ = try fixture.workspace.setStatus(.wontDo, of: subtask.id, in: fixture.context)
            fixture.advance(days: 1)
            _ = try fixture.workspace.setStatus(.done, of: parent.id, in: fixture.context)
            try expectEqual(fixture.get(parent).status, .done)
            try expectEqual(fixture.get(parent).closedAt, fixture.now)
        },
        TestCase("given an idle In Progress task, when it's set In Progress again, then it's no longer stalled") {
            let fixture = try Fixture()
            let task = try fixture.task("Task", status: .inProgress)
            fixture.advance(days: 10)
            try expectEqual(
                fixture.get(task).procrastination(now: fixture.now, settings: Settings()), .inactive(days: 10))
            _ = try fixture.workspace.setStatus(.inProgress, of: task.id, in: fixture.context)
            try expectEqual(fixture.get(task).procrastination(now: fixture.now, settings: Settings()), nil)
        },
        TestCase("given a closed task, when it reopens, then closedAt is cleared") {
            let fixture = try Fixture()
            let task = try fixture.task("Task", status: .done)
            _ = try fixture.workspace.setStatus(.todo, of: task.id, in: fixture.context)
            try expectEqual(fixture.get(task).closedAt, nil)
        },
        TestCase("given open Sub tasks, when the parent moves to Won't Do without cascade, then it asks to confirm") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent")
            let subtask = try fixture.task("Sub task", parent: parent)
            try expectError(.wontDoNeedsCascade(task: parent.id, open: [subtask.id])) {
                try fixture.workspace.setStatus(.wontDo, of: parent.id, in: fixture.context)
            }
            try expectEqual(fixture.get(subtask).status, .todo)
        },
        TestCase("given open Sub tasks, when the parent moves to Won't Do with cascade, then they close too") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent")
            let open = try fixture.task("Open", parent: parent)
            let done = try fixture.task("Done", status: .done, parent: parent)
            let change = try fixture.workspace.setStatus(.wontDo, of: parent.id, cascade: true, in: fixture.context)
            try expectEqual(fixture.get(parent).status, .wontDo)
            try expectEqual(fixture.get(open).status, .wontDo)
            try expectEqual(fixture.get(done).status, .done)
            try expect(change.events.contains { $0.taskID == open.id && $0.reason == .closedWithParent })
        },
        TestCase("given a closed parent, when a Sub task is added, then the parent is In Progress again") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent", status: .done)
            fixture.advance(days: 1)
            let subtask = Workspace.NewTask(title: "One more thing", parentID: parent.id)
            let change = try fixture.workspace.createTask(subtask, in: fixture.context)
            try expectEqual(fixture.get(parent).status, .inProgress)
            try expectEqual(change.reopenedParent, parent.id)
            let reopen = change.events.first { $0.taskID == parent.id && $0.field == .status }
            try expectEqual(reopen?.actor.kind, .system)
            try expectEqual(reopen?.reason, .subtaskOpenedUnderClosedParent)
        },
        TestCase("given a parent reopened by a new Sub task, when the change is undone, then both are as before") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent", status: .done)
            let change = try fixture.workspace.createTask(
                Workspace.NewTask(title: "One more thing", parentID: parent.id), in: fixture.context)
            _ = fixture.workspace.undo(change, in: fixture.context)
            try expectEqual(fixture.get(parent), parent)
            try expectEqual(fixture.workspace.subtasks(of: parent.id), [])
        },
        TestCase("given a closed parent, when a closed Sub task reopens, then the parent is In Progress again") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent")
            let subtask = try fixture.task("Sub task", status: .done, parent: parent)
            _ = try fixture.workspace.setStatus(.done, of: parent.id, in: fixture.context)
            let change = try fixture.workspace.setStatus(.todo, of: subtask.id, in: fixture.context)
            try expectEqual(fixture.get(parent).status, .inProgress)
            try expectEqual(change.reopenedParent, parent.id)
        },
        TestCase("given a closed parent, when a closed Sub task is added, then the parent stays closed") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent", status: .done)
            try fixture.task("Already done", status: .done, parent: parent)
            try expectEqual(fixture.get(parent).status, .done)
        },
        TestCase("given any status, then it can move to any other unless a rule applies") {
            let fixture = try Fixture()
            let task = try fixture.task("Task")
            for status in [TaskStatus.done, .procrastinated, .wontDo, .inProgress, .todo, .done] {
                _ = try fixture.workspace.setStatus(status, of: task.id, in: fixture.context)
                try expectEqual(fixture.get(task).status, status)
            }
            let statusEvents = fixture.workspace.history.filter { $0.taskID == task.id && $0.field == .status }
            try expectEqual(statusEvents.count, 6)
        },
    ]
}
