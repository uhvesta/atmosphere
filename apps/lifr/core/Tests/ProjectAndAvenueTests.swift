import Foundation
import LifrCore
import TestingKit

func projectAndAvenueTests() -> [TestCase] {
    [
        TestCase("given a Project, when it's edited, then only the given fields change") {
            let fixture = try Fixture()
            let id = fixture.marathon.id
            let friday = try CalendarDate("2026-11-06")
            try fixture.workspace.edit(project: id, Workspace.ProjectEdit(name: "  Spring half  ", dueDate: friday))
            try fixture.workspace.edit(project: id, Workspace.ProjectEdit(details: "Sub 2 hours"))
            let project = try fixture.workspace.projects[id].unwrap("the Project is gone")
            try expectEqual(project.name, "Spring half")
            try expectEqual(project.details, "Sub 2 hours")
            try expectEqual(project.dueDate, friday)
            try fixture.workspace.edit(project: id, Workspace.ProjectEdit(dueDate: .some(nil)))
            try expectEqual(fixture.workspace.projects[id]?.dueDate, nil)
        },
        TestCase("given an empty or too long name, then a Project or Avenue keeps its old one") {
            let fixture = try Fixture()
            try expectError(.invalidName(length: 0)) {
                try fixture.workspace.edit(project: fixture.marathon.id, Workspace.ProjectEdit(name: "  "))
            }
            try expectError(.invalidName(length: 101)) {
                try fixture.workspace.edit(
                    avenue: fixture.health.id, Workspace.AvenueEdit(name: String(repeating: "x", count: 101)))
            }
            try expectEqual(fixture.workspace.projects[fixture.marathon.id]?.name, "Half marathon")
            try expectEqual(fixture.workspace.avenues[fixture.health.id]?.name, "Health")
        },
        TestCase("given a Project with tasks, when it moves Avenue, then its tasks and Sub tasks move with it") {
            let fixture = try Fixture()
            let parent = try fixture.task("Long run", project: fixture.marathon)
            let subtask = try fixture.task("Stretch", parent: parent)
            let trashed = try fixture.task("Old plan", project: fixture.marathon)
            _ = try fixture.workspace.moveToTrash(trashed.id, in: fixture.context)
            let outside = try fixture.task("Book physio")
            let sport = try fixture.workspace.createAvenue(key: "SPORT", name: "Sport", in: fixture.context)
            _ = try fixture.workspace.move(project: fixture.marathon.id, toAvenue: sport.id, in: fixture.context)
            try expectEqual(fixture.workspace.projects[fixture.marathon.id]?.avenueID, sport.id)
            for task in [parent, subtask, trashed] {
                try expectEqual(fixture.get(task).avenueID, sport.id)
                try expectEqual(fixture.get(task).projectID, fixture.marathon.id)
            }
            try expectEqual(fixture.get(outside).avenueID, fixture.health.id)
        },
        TestCase("given open tasks, when a Project is archived moving them, then they stay in its Avenue") {
            let fixture = try Fixture()
            let parent = try fixture.task("Long run", project: fixture.marathon)
            let subtask = try fixture.task("Stretch", parent: parent)
            let done = try fixture.task("Buy shoes", status: .done, project: fixture.marathon)
            _ = try fixture.workspace.archive(
                project: fixture.marathon.id, openTasks: .moveToAvenue, in: fixture.context)
            try expectEqual(fixture.workspace.projects[fixture.marathon.id]?.status, .archived)
            for task in [parent, subtask] {
                try expectEqual(fixture.get(task).projectID, nil)
                try expectEqual(fixture.get(task).avenueID, fixture.health.id)
                try expectEqual(fixture.get(task).status, .todo)
            }
            try expectEqual(fixture.get(done).projectID, fixture.marathon.id)
        },
        TestCase("given open tasks, when a Project is archived closing them, then they and their Sub tasks close") {
            let fixture = try Fixture()
            let parent = try fixture.task("Long run", project: fixture.marathon)
            let subtask = try fixture.task("Stretch", parent: parent)
            _ = try fixture.workspace.archive(
                project: fixture.marathon.id, openTasks: .closeAsWontDo, in: fixture.context)
            try expectEqual(fixture.get(parent).status, .wontDo)
            try expectEqual(fixture.get(subtask).status, .wontDo)
            try expectEqual(fixture.get(parent).projectID, fixture.marathon.id)
        },
        TestCase("given an Avenue with open tasks, then archiving it needs them closed") {
            let fixture = try Fixture()
            let task = try fixture.task("Book physio")
            try fixture.task("Old", status: .done)
            try expectError(.avenueHasOpenTasks(avenue: fixture.health.id, open: [task.id])) {
                try fixture.workspace.archive(avenue: fixture.health.id, openTasks: nil, in: fixture.context)
            }
            try expectEqual(fixture.workspace.avenues[fixture.health.id]?.archivedAt, nil)
            _ = try fixture.workspace.archive(
                avenue: fixture.health.id, openTasks: .closeAsWontDo, in: fixture.context)
            try expectEqual(fixture.get(task).status, .wontDo)
            try expectEqual(fixture.workspace.avenues[fixture.health.id]?.archivedAt, fixture.now)
        },
        TestCase("given an archived Avenue, when it's restored, then it and its tasks are as they were") {
            let fixture = try Fixture()
            let task = try fixture.task("Old", status: .done)
            _ = try fixture.workspace.archive(avenue: fixture.health.id, openTasks: nil, in: fixture.context)
            try fixture.workspace.restore(avenue: fixture.health.id)
            try expectEqual(fixture.workspace.avenues[fixture.health.id]?.archivedAt, nil)
            try expectEqual(fixture.get(task).status, .done)
        },
        TestCase("given an Avenue, when it's edited, then its name, color and symbol change") {
            let fixture = try Fixture()
            try fixture.workspace.edit(
                avenue: fixture.health.id, Workspace.AvenueEdit(name: "Body", color: "green", symbol: "heart"))
            let avenue = try fixture.workspace.avenues[fixture.health.id].unwrap("the Avenue is gone")
            try expectEqual([avenue.name, avenue.color, avenue.symbol], ["Body", "green", "heart"])
            try expectEqual(avenue.key.rawValue, "HEALTH")
        },
        TestCase("given an archived Project, when the change is reverted, then it and its tasks are as before") {
            let fixture = try Fixture()
            let task = try fixture.task("Long run", project: fixture.marathon)
            let change = try fixture.workspace.perform(in: fixture.context) { (workspace, context) throws(LifrError) in
                try workspace.edit(project: fixture.marathon.id, Workspace.ProjectEdit(name: "Spring half"))
                return try workspace.archive(project: fixture.marathon.id, openTasks: .moveToAvenue, in: context)
            }
            let entry = try change.entry.unwrap("the archive wasn't journaled")
            try fixture.workspace.perform(in: fixture.context) { (workspace, context) throws(LifrError) in
                try workspace.revert([entry], in: context)
            }
            let project = try fixture.workspace.projects[fixture.marathon.id].unwrap("the Project is gone")
            try expectEqual(project.status, .active)
            try expectEqual(project.name, "Half marathon")
            try expectEqual(fixture.get(task).projectID, fixture.marathon.id)
        },
    ]
}
