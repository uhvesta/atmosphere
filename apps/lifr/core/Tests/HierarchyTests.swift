import Foundation
import LifrCore
import TestingKit

func hierarchyTests() -> [TestCase] {
    [
        TestCase("given a title of 0 or 201 characters, then the task is refused") {
            let fixture = try Fixture()
            try expectError(.invalidTitle(length: 0)) { try fixture.task("   ") }
            try expectError(.invalidTitle(length: 201)) { try fixture.task(String(repeating: "x", count: 201)) }
        },
        TestCase("given a Project in another Avenue, then a task can't use it") {
            let fixture = try Fixture()
            let home = try fixture.workspace.createAvenue(key: "HOME", name: "Home", in: fixture.context)
            var new = Workspace.NewTask(title: "Fix the sink", avenueID: home.id, projectID: fixture.marathon.id)
            new.priority = .high
            try expectError(.projectInDifferentAvenue(project: fixture.marathon.id, avenue: home.id)) {
                try fixture.workspace.createTask(new, in: fixture.context)
            }
        },
        TestCase("given a parent task, then a Sub task takes its Avenue and Project") {
            let fixture = try Fixture()
            let parent = try fixture.task("Run a half marathon", project: fixture.marathon)
            let subtask = try fixture.task("Get a gait analysis", parent: parent)
            try expectEqual(subtask.avenueID, fixture.health.id)
            try expectEqual(subtask.projectID, fixture.marathon.id)
            try expectEqual(fixture.workspace.subtasks(of: parent.id).map(\.id), [subtask.id])
        },
        TestCase("given a Sub task, then it can't have its own Sub tasks") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent")
            let subtask = try fixture.task("Sub task", parent: parent)
            try expectError(.parentIsSubtask(subtask.id)) { try fixture.task("Nested", parent: subtask) }
        },
        TestCase("given a task with Sub tasks, then it can't move under another task") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent")
            try fixture.task("Sub task", parent: parent)
            let other = try fixture.task("Other")
            try expectError(.taskHasSubtasks(parent.id)) {
                try fixture.workspace.setParent(other.id, of: parent.id, in: fixture.context)
            }
        },
        TestCase("given a Sub task, when it's promoted, then it's a top-level task in the same place") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent", project: fixture.marathon)
            let subtask = try fixture.task("Sub task", parent: parent)
            _ = try fixture.workspace.setParent(nil, of: subtask.id, in: fixture.context)
            try expectEqual(fixture.get(subtask).parentID, nil)
            try expectEqual(fixture.get(subtask).projectID, fixture.marathon.id)
        },
        TestCase("given a parent with Sub tasks, when it moves Avenue, then its Sub tasks move too") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent", project: fixture.marathon)
            let subtask = try fixture.task("Sub task", parent: parent)
            let home = try fixture.workspace.createAvenue(key: "HOME", name: "Home", in: fixture.context)
            _ = try fixture.workspace.move(parent.id, toAvenue: home.id, project: nil, in: fixture.context)
            try expectEqual(fixture.get(subtask).avenueID, home.id)
            try expectEqual(fixture.get(subtask).projectID, nil)
            try expectEqual(try fixture.workspace.task(code: subtask.code.rawValue).id, subtask.id)
        },
        TestCase("given a trashed Sub task, when its parent moves, then it moves too and is restored with it") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent")
            let trashed = try fixture.task("Old step", parent: parent)
            _ = try fixture.workspace.moveToTrash(trashed.id, in: fixture.context)
            let home = try fixture.workspace.createAvenue(key: "HOME", name: "Home", in: fixture.context)
            let garden = try fixture.workspace.createProject(name: "Garden", inAvenue: home.id, in: fixture.context)
            _ = try fixture.workspace.move(parent.id, toAvenue: home.id, project: garden.id, in: fixture.context)
            _ = try fixture.workspace.restore(trashed.id, in: fixture.context)
            try expectEqual(fixture.get(trashed).avenueID, home.id)
            try expectEqual(fixture.get(trashed).projectID, garden.id)
        },
        TestCase("given a moved task with Sub tasks, when the move is undone, then they all go back") {
            let fixture = try Fixture()
            let parent = try fixture.task("Parent", project: fixture.marathon)
            let subtask = try fixture.task("Sub task", parent: parent)
            let home = try fixture.workspace.createAvenue(key: "HOME", name: "Home", in: fixture.context)
            let change = try fixture.workspace.move(parent.id, toAvenue: home.id, project: nil, in: fixture.context)
            _ = fixture.workspace.undo(change, in: fixture.context)
            for task in [parent, subtask] {
                try expectEqual(fixture.get(task).avenueID, fixture.health.id)
                try expectEqual(fixture.get(task).projectID, fixture.marathon.id)
            }
        },
        TestCase("given a Sub task, then it can't be moved on its own") {
            let fixture = try Fixture()
            let subtask = try fixture.task("Sub task", parent: try fixture.task("Parent"))
            try expectError(.subtaskFollowsParent(subtask.id)) {
                try fixture.workspace.move(subtask.id, toAvenue: fixture.health.id, project: nil, in: fixture.context)
            }
        },
        TestCase("given a Project with an open task, then it can't be completed but can be archived") {
            let fixture = try Fixture()
            let task = try fixture.task("Long run", project: fixture.marathon)
            try expectError(.projectHasOpenTasks(project: fixture.marathon.id, open: [task.id])) {
                try fixture.workspace.setStatus(.completed, ofProject: fixture.marathon.id, in: fixture.context)
            }
            try fixture.workspace.setStatus(.archived, ofProject: fixture.marathon.id, in: fixture.context)
            try expectEqual(fixture.workspace.projects[fixture.marathon.id]?.status, .archived)
        },
        TestCase("given a Project whose tasks are all closed, then it can be completed") {
            let fixture = try Fixture()
            try fixture.task("Long run", status: .done, project: fixture.marathon)
            try fixture.workspace.setStatus(.completed, ofProject: fixture.marathon.id, in: fixture.context)
            try expectEqual(fixture.workspace.projects[fixture.marathon.id]?.completedAt, fixture.now)
        },
        TestCase("given an edit, then each changed field writes one history event") {
            let fixture = try Fixture()
            let task = try fixture.task("Buy shoes")
            let change = try fixture.workspace.edit(
                task.id, .init(title: "Buy running shoes", priority: .high), in: fixture.context)
            try expectEqual(change.events.map(\.field), [.title, .priority])
            try expectEqual(change.events.map(\.newValue), ["Buy running shoes", "high"])
        },
    ]
}
