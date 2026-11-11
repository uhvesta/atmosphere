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
        TestCase("given no Inbox, when one is ensured, then it's added first in the order, once") {
            let fixture = try Fixture()
            try expectEqual(fixture.workspace.ensureInbox(in: fixture.context), true)
            try expectEqual(fixture.workspace.ensureInbox(in: fixture.context), false)
            let order = fixture.workspace.avenues.values.sorted { $0.sortOrder < $1.sortOrder }
            try expectEqual(order.map(\.isInbox), [true, false])
            try expectEqual(order.first?.key.rawValue, "INBOX")
        },
        TestCase("given an Avenue keyed INBOX already, when the Inbox is added, then it takes another key") {
            let fixture = try Fixture()
            try fixture.workspace.createAvenue(key: "INBOX", name: "Mine", in: fixture.context)
            fixture.workspace.ensureInbox(in: fixture.context)
            let inbox = try fixture.workspace.avenues[Avenue.inboxID].unwrap("no Inbox")
            try expectEqual(inbox.key.needsRename, true)
        },
        TestCase("given the Inbox, when it's archived, then that's refused and it stays") {
            let fixture = try Fixture()
            fixture.workspace.ensureInbox(in: fixture.context)
            try expectError(.inboxCannotBeArchived) {
                try fixture.workspace.archive(avenue: Avenue.inboxID, openTasks: .closeAsWontDo, in: fixture.context)
            }
            try expectEqual(fixture.workspace.avenues[Avenue.inboxID]?.archivedAt, nil)
        },
        TestCase("given two devices that each made an Inbox, when they sync, then one Inbox holds both tasks") {
            let phone = try Fixture()
            let mac = Fixture(copying: phone, device: "Mac")
            var tasks: [UUID] = []
            for device in [phone, mac] {
                device.workspace.ensureInbox(in: device.context)
                let change = try device.workspace.createTask(
                    Workspace.NewTask(title: "Call \(tasks.count)", avenueID: Avenue.inboxID), in: device.context)
                try tasks.append(change.previous.first { $0.value == nil }.unwrap("no new task").key)
            }
            let macTask = try mac.workspace.tasks[tasks[1]].unwrap("no Mac task")
            let macInbox = try mac.workspace.avenues[Avenue.inboxID].unwrap("no Mac Inbox")
            phone.workspace.importSynced(records: [
                RecordID(.avenue, Avenue.inboxID): .avenue(macInbox), RecordID(.task, macTask.id): .task(macTask),
            ])
            _ = phone.workspace.reconcile(in: phone.context)
            try expectEqual(phone.workspace.avenues.values.filter { $0.name == "Inbox" }.count, 1)
            try expectEqual(Set(tasks.compactMap { phone.workspace.tasks[$0]?.avenueID }), [Avenue.inboxID])
        },
        TestCase("given three tasks in manual order, when the third moves to the top, then the order is 3, 1, 2") {
            let fixture = try Fixture()
            let tasks = try ["One", "Two", "Three"].map { try fixture.task($0) }
            let change = try fixture.workspace.reorder(
                task: tasks[2].id, after: nil, before: tasks[0].id, in: fixture.context)
            let order = try tasks.map { try fixture.get($0) }
                .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
            try expectEqual(order.map(\.title), ["Three", "One", "Two"])
            try expectEqual(Set(change.previous.keys), [tasks[2].id])
        },
        TestCase("given a task moved between two others, when it's undone, then it's back where it was") {
            let fixture = try Fixture()
            let tasks = try ["One", "Two", "Three"].map { try fixture.task($0) }
            let change = try fixture.workspace.reorder(
                task: tasks[0].id, after: tasks[1].id, before: tasks[2].id, in: fixture.context)
            _ = fixture.workspace.undo(change, in: fixture.context)
            try expectEqual(fixture.get(tasks[0]).sortOrder, tasks[0].sortOrder)
        },
        TestCase("given two tasks with the same key, when one moves between them, then it goes after both") {
            let fixture = try Fixture()
            let tasks = try ["One", "Two", "Three"].map { try fixture.task($0) }
            var tied = tasks[1]
            tied.sortOrder = tasks[0].sortOrder
            fixture.workspace.importSynced(records: [RecordID(.task, tied.id): .task(tied)])
            _ = try fixture.workspace.reorder(
                task: tasks[2].id, after: tasks[0].id, before: tasks[1].id, in: fixture.context)
            try expectEqual(fixture.get(tasks[2]).sortOrder > tasks[0].sortOrder, true)
        },
        TestCase("given Avenues, when the last moves to the top, then it sorts first") {
            let fixture = try Fixture()
            let home = try fixture.workspace.createAvenue(key: "HOME", name: "Home", in: fixture.context)
            try fixture.workspace.reorder(avenue: home.id, after: nil, before: fixture.health.id)
            let order = fixture.workspace.avenues.values.sorted { $0.sortOrder < $1.sortOrder }.map(\.name)
            try expectEqual(order, ["Home", "Health"])
        },
        TestCase("given a missing neighbour, then reordering is refused and changes nothing") {
            let fixture = try Fixture()
            let task = try fixture.task("One")
            let missing = UUID(sequence: 999)
            try expectError(.notFound(.task, missing.uuidString)) {
                try fixture.workspace.reorder(task: task.id, after: missing, before: nil, in: fixture.context)
            }
            try expectEqual(fixture.get(task).sortOrder, task.sortOrder)
        },
    ]
}
