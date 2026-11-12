import Foundation
import LifrCore
import TestingKit

extension Fixture {
    /// Makes `edit` to `tasks` the way the stores do, with one journal entry.
    func apply(_ edit: BulkEdit, to tasks: [LifrTask], cascade: Bool = false) throws -> Change {
        try perform { workspace, context throws(LifrError) in
            try workspace.apply(edit, to: tasks.map(\.id), cascade: cascade, in: context)
        }
    }
}

func bulkEditTests() -> [TestCase] {
    [
        TestCase("given three tasks, when they're set Done at once, then all are Done in one journal entry") {
            let fixture = try Fixture()
            let tasks = try ["Run", "Swim", "Ride"].map { try fixture.task($0) }
            let entries = fixture.workspace.journal.count
            _ = try fixture.apply(.status(.done), to: tasks)
            for task in tasks {
                try expectEqual(try fixture.get(task).status, .done)
            }
            try expectEqual(fixture.workspace.journal.count, entries + 1)
        },
        TestCase("given a bulk change, when it's undone, then every task is back as it was") {
            let fixture = try Fixture()
            let run = try fixture.task("Run")
            let swim = try fixture.task("Swim")
            let change = try fixture.apply(.priority(.high), to: [run, swim])
            _ = fixture.workspace.undo(change, in: fixture.context)
            try expectEqual(try fixture.get(run).priority, run.priority)
            try expectEqual(try fixture.get(swim).priority, swim.priority)
        },
        TestCase("given one task with open Sub tasks, when Done is set on it and others, then none change") {
            let fixture = try Fixture()
            let race = try fixture.task("Race")
            let bib = try fixture.task("Pin the bib", parent: race)
            let run = try fixture.task("Run")
            try expectError(LifrError.openSubtasks(task: race.id, open: [bib.id])) {
                _ = try fixture.apply(.status(.done), to: [run, race])
            }
            try expectEqual(try fixture.get(run).status, .todo)
            try expectEqual(try fixture.get(race).status, .todo)
        },
        TestCase("given the person agreed, when Done is set with open Sub tasks, then those are Won't Do") {
            let fixture = try Fixture()
            let race = try fixture.task("Race")
            let bib = try fixture.task("Pin the bib", parent: race)
            let run = try fixture.task("Run")
            _ = try fixture.apply(.status(.done), to: [run, race], cascade: true)
            try expectEqual(try fixture.get(race).status, .done)
            try expectEqual(try fixture.get(run).status, .done)
            try expectEqual(try fixture.get(bib).status, .wontDo)
        },
        TestCase("given a parent picked with its Sub task, when both are set Done, then both are Done") {
            let fixture = try Fixture()
            let race = try fixture.task("Race")
            let bib = try fixture.task("Pin the bib", parent: race)
            _ = try fixture.apply(.status(.done), to: [race, bib])
            try expectEqual(try fixture.get(race).status, .done)
            try expectEqual(try fixture.get(bib).status, .done)
        },
        TestCase("given tasks, when they're given a due date and then none, then each has it and then none") {
            let fixture = try Fixture()
            let run = try fixture.task("Run")
            let swim = try fixture.task("Swim")
            let date = try CalendarDate("2026-11-10")
            _ = try fixture.apply(.dueDate(date), to: [run, swim])
            try expectEqual(try fixture.get(run).dueDate, date)
            try expectEqual(try fixture.get(swim).dueDate, date)
            _ = try fixture.apply(.dueDate(nil), to: [run, swim])
            try expectEqual(try fixture.get(run).dueDate, nil)
        },
        TestCase("given a timed task and one over days, when rescheduled and then none, then they move and clear") {
            let fixture = try Fixture()
            let call = try fixture.task("Call")
            let trip = try fixture.task("Trip")
            let start = try CalendarDate("2026-11-10")
            let end = try CalendarDate("2026-11-12")
            _ = try fixture.workspace.edit(
                call.id, .init(dueDate: .some(start), dueTime: .some(try TimeOfDay("15:00"))),
                in: fixture.context)
            _ = try fixture.workspace.edit(
                trip.id, .init(startDate: .some(start), dueDate: .some(end)), in: fixture.context)
            _ = try fixture.apply(.dueDate(try CalendarDate("2026-11-15")), to: [call, trip])
            try expectEqual(try fixture.get(trip).startDate, try CalendarDate("2026-11-13"))
            try expectEqual(try fixture.get(call).dueTime, try TimeOfDay("15:00"))
            _ = try fixture.apply(.dueDate(nil), to: [call, trip])
            try expectEqual(try fixture.get(call).dueDate, nil)
            try expectEqual(try fixture.get(call).dueTime, nil)
        },
        TestCase("given tasks, one with the label, when it's added to all, then each has it once") {
            let fixture = try Fixture()
            let errand = fixture.workspace.createLabel(name: "Errand", in: fixture.context)
            let run = try fixture.task("Run")
            _ = try fixture.workspace.edit(run.id, .init(labelIDs: [errand.id]), in: fixture.context)
            let swim = try fixture.task("Swim")
            _ = try fixture.apply(.addLabel(errand.id), to: [run, swim])
            try expectEqual(try fixture.get(run).labelIDs, [errand.id])
            try expectEqual(try fixture.get(swim).labelIDs, [errand.id])
        },
        TestCase("given a parent picked with its Sub task, when they're moved, then both are in the Project") {
            let fixture = try Fixture()
            let race = try fixture.task("Race")
            let bib = try fixture.task("Pin the bib", parent: race)
            let run = try fixture.task("Run")
            _ = try fixture.apply(.move(avenue: fixture.health.id, project: fixture.marathon.id), to: [race, bib, run])
            for task in [race, bib, run] {
                try expectEqual(try fixture.get(task).projectID, fixture.marathon.id)
            }
        },
        TestCase("given a Sub task picked without its parent, when it's moved, then it's refused") {
            let fixture = try Fixture()
            let race = try fixture.task("Race")
            let bib = try fixture.task("Pin the bib", parent: race)
            try expectError(LifrError.subtaskFollowsParent(bib.id)) {
                _ = try fixture.apply(.move(avenue: fixture.health.id, project: fixture.marathon.id), to: [bib])
            }
        },
        TestCase("given a parent picked with its Sub task, when they're trashed, then both are in the trash") {
            let fixture = try Fixture()
            let race = try fixture.task("Race")
            let bib = try fixture.task("Pin the bib", parent: race)
            _ = try fixture.apply(.trash, to: [bib, race])
            try expect(try fixture.get(race).deletedAt != nil)
            try expect(try fixture.get(bib).deletedAt != nil)
        },
        TestCase("given a missing task among them, when they're changed, then none change") {
            let fixture = try Fixture()
            let run = try fixture.task("Run")
            let missing = UUID()
            try expectError(LifrError.notFound(.task, missing.uuidString)) {
                _ = try fixture.perform { workspace, context throws(LifrError) in
                    try workspace.apply(.priority(.high), to: [run.id, missing], in: context)
                }
            }
            try expectEqual(try fixture.get(run).priority, run.priority)
        },
    ]
}
