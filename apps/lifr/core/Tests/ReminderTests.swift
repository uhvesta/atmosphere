import Foundation
import LifrCore
import TestingKit

func reminderTests() -> [TestCase] {
    [
        TestCase("given a set time, then the reminder fires then") {
            let fixture = try Fixture()
            let task = try fixture.task("Call the plumber")
            let at = fixture.now + 3600
            let reminder = try fixture.workspace.addReminder(.at(at), to: task.id, in: fixture.context)
            try expectEqual(reminder.fireDate(for: fixture.get(task), in: .gmt), at)
        },
        TestCase("given a due date and time, then a reminder before it fires that long before") {
            let fixture = try Fixture()
            let task = try fixture.task("Long run")
            _ = try fixture.workspace.edit(
                task.id, Workspace.TaskEdit(dueDate: try CalendarDate("2026-11-05"), dueTime: try TimeOfDay("07:30")),
                in: fixture.context)
            let reminder = try fixture.workspace.addReminder(
                .beforeDue(seconds: 3600), to: task.id, in: fixture.context)
            try expectEqual(
                reminder.fireDate(for: fixture.get(task), in: .gmt), date("2026-11-05T06:30:00Z"))
        },
        TestCase("given a due date with no time, then it counts from 9:00 that day in the time zone") {
            let fixture = try Fixture()
            let task = try fixture.task("Renew passport")
            _ = try fixture.workspace.edit(
                task.id, Workspace.TaskEdit(dueDate: try CalendarDate("2026-11-05")), in: fixture.context)
            let reminder = try fixture.workspace.addReminder(
                .beforeDue(seconds: 24 * 3600), to: task.id, in: fixture.context)
            let berlin = try TimeZone(identifier: "Europe/Berlin").unwrap()
            try expectEqual(
                reminder.fireDate(for: fixture.get(task), in: berlin), date("2026-11-04T08:00:00Z"))
        },
        TestCase("given no due date, then a reminder before it is refused") {
            let fixture = try Fixture()
            let task = try fixture.task("Someday")
            try expectError(.reminderNeedsDueDate(task.id)) {
                try fixture.workspace.addReminder(.beforeDue(seconds: 60), to: task.id, in: fixture.context)
            }
        },
        TestCase("given a task in the trash, then a reminder is refused") {
            let fixture = try Fixture()
            let task = try fixture.task("Gone")
            _ = try fixture.workspace.moveToTrash(task.id, in: fixture.context)
            try expectError(.inTrash(task.id)) {
                try fixture.workspace.addReminder(.at(fixture.now), to: task.id, in: fixture.context)
            }
        },
        TestCase("given reminders, then a task lists them soonest first, and removing one drops it") {
            let fixture = try Fixture()
            let task = try fixture.task("Task")
            let later = try fixture.workspace.addReminder(.at(fixture.now + 7200), to: task.id, in: fixture.context)
            let sooner = try fixture.workspace.addReminder(.at(fixture.now + 60), to: task.id, in: fixture.context)
            try expectEqual(fixture.workspace.reminders(of: task.id, timeZone: .gmt).map(\.id), [sooner.id, later.id])
            try fixture.workspace.removeReminder(sooner.id)
            try expectEqual(fixture.workspace.reminders(of: task.id, timeZone: .gmt).map(\.id), [later.id])
            try expectError(.notFound(.reminder, sooner.id.uuidString)) {
                try fixture.workspace.removeReminder(sooner.id)
            }
        },
    ]
}

private func date(_ string: String) -> Date? {
    ISO8601DateFormatter().date(from: string)
}
