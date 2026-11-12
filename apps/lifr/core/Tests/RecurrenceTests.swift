import Foundation
import LifrCore
import TestingKit

extension Fixture {
    /// A task due `due` that repeats by `rule`.
    func repeating(
        _ title: String, due: String?, _ rule: Recurrence.Rule, every interval: Int = 1
    ) throws -> LifrTask {
        let task = try self.task(title)
        if let due {
            _ = try workspace.edit(task.id, .init(dueDate: .some(try CalendarDate(due))), in: context)
        }
        _ = try workspace.setRecurrence(rule, every: interval, of: task.id, in: context)
        return try get(task)
    }

    /// Moves the clock to noon UTC on `day`.
    func set(today day: String) throws {
        now = try CalendarDate(day).noonUTC
    }

    /// The task in `task`'s series after it.
    func next(after task: LifrTask) throws -> LifrTask {
        let index = try task.recurrence.unwrap("\(task.title) doesn't repeat").index
        let series = task.recurrence?.seriesID
        let next = workspace.tasks.values.first {
            $0.recurrence?.seriesID == series && $0.recurrence?.index == index + 1
        }
        return try next.unwrap("\(task.title) has no next occurrence")
    }
}

private func day(_ string: String) throws -> CalendarDate { try CalendarDate(string) }

func recurrenceTests() -> [TestCase] {
    [
        TestCase("given a weekly Monday task, when it's done on a Wednesday, then the next is due next Monday") {
            let fixture = try Fixture()
            let task = try fixture.repeating("Weigh in", due: "2026-11-09", .weekly(weekdays: [2]))
            try fixture.set(today: "2026-11-11")
            _ = try fixture.workspace.setStatus(.done, of: task.id, in: fixture.context)
            try expectEqual(try fixture.next(after: task).dueDate, try day("2026-11-16"))
            try expectEqual(try fixture.next(after: task).status, .todo)
        },
        TestCase("given monthly on the 31st, then a 30-day month has it on the 30th and the next on the 31st") {
            let fixture = try Fixture()
            let task = try fixture.repeating("Pay rent", due: "2026-10-31", .monthly(day: 31))
            try fixture.set(today: "2026-10-31")
            _ = try fixture.workspace.setStatus(.done, of: task.id, in: fixture.context)
            let november = try fixture.next(after: task)
            try expectEqual(november.dueDate, try day("2026-11-30"))
            try fixture.set(today: "2026-11-30")
            _ = try fixture.workspace.setStatus(.done, of: november.id, in: fixture.context)
            try expectEqual(try fixture.next(after: november).dueDate, try day("2026-12-31"))
        },
        TestCase("given repeating after closing, then the next counts from the closing day") {
            let fixture = try Fixture()
            let task = try fixture.repeating("Water plants", due: "2026-11-01", .afterClosing(.day), every: 3)
            try fixture.set(today: "2026-11-11")
            _ = try fixture.workspace.setStatus(.wontDo, of: task.id, in: fixture.context)
            try expectEqual(try fixture.next(after: task).dueDate, try day("2026-11-14"))
        },
        TestCase("given an overdue daily task, when it's done, then the next is due tomorrow") {
            let fixture = try Fixture()
            let task = try fixture.repeating("Stretch", due: "2026-11-01", .daily)
            try fixture.set(today: "2026-11-11")
            _ = try fixture.workspace.setStatus(.done, of: task.id, in: fixture.context)
            try expectEqual(try fixture.next(after: task).dueDate, try day("2026-11-12"))
        },
        TestCase("given schedules, then each next day is the expected one") {
            let cases: [(Recurrence.Rule, Int, String, String)] = [
                (.daily, 2, "2026-11-11", "2026-11-13"),
                (.weekly(weekdays: [2, 3, 4, 5, 6]), 1, "2026-11-13", "2026-11-16"),
                (.weekly(weekdays: [2, 5]), 2, "2026-11-12", "2026-11-23"),
                (.weekly(weekdays: [2, 5]), 2, "2026-11-09", "2026-11-12"),
                (.monthlyWeekday(ordinal: 1, weekday: 2), 1, "2026-11-11", "2026-12-07"),
                (.monthlyWeekday(ordinal: -1, weekday: 6), 1, "2026-11-01", "2026-11-27"),
                (.yearly(month: 2, day: 29), 1, "2027-03-01", "2028-02-29"),
                (.yearly(month: 2, day: 29), 1, "2026-03-01", "2027-02-28"),
                (.afterClosing(.month), 1, "2027-01-31", "2027-02-28"),
            ]
            for (rule, interval, from, expected) in cases {
                let recurrence = Recurrence(rule: rule, interval: interval, seriesID: UUID(sequence: 1))
                let next = recurrence.nextDue(after: try day(from), closedOn: try day(from))
                try expectEqual("\(recurrence) after \(from): \(next)", "\(recurrence) after \(from): \(expected)")
            }
        },
        TestCase("given a repeating task with Sub tasks and a reminder, then the next has them open and reminded") {
            let fixture = try Fixture()
            let task = try fixture.repeating("Long run", due: "2026-11-08", .weekly(weekdays: [1]))
            let shoes = try fixture.task("Lace shoes", parent: task)
            _ = try fixture.workspace.setStatus(.done, of: shoes.id, in: fixture.context)
            try fixture.workspace.addReminder(.beforeDue(seconds: 3600), to: task.id, in: fixture.context)
            try fixture.set(today: "2026-11-08")
            _ = try fixture.workspace.setStatus(.done, of: task.id, in: fixture.context)
            let next = try fixture.next(after: task)
            let subtasks = fixture.workspace.subtasks(of: next.id)
            try expectEqual(subtasks.map(\.title), ["Lace shoes"])
            try expectEqual(subtasks.map(\.status), [.todo])
            try expectEqual(
                fixture.workspace.reminders(of: next.id, timeZone: .gmt).map(\.trigger), [.beforeDue(seconds: 3600)])
            try expectEqual(try fixture.get(task).status, .done)
        },
        TestCase("given a repeating task with a snooze, then the next keeps only the reminders that were set") {
            let fixture = try Fixture()
            let task = try fixture.repeating("Long run", due: "2026-11-08", .weekly(weekdays: [1]))
            try fixture.workspace.addReminder(.beforeDue(seconds: 0), to: task.id, in: fixture.context)
            let snooze = Reminder.Trigger.at(fixture.now + 3600)
            try fixture.workspace.addReminder(snooze, to: task.id, isOneOff: true, in: fixture.context)
            try fixture.set(today: "2026-11-08")
            _ = try fixture.workspace.setStatus(.done, of: task.id, in: fixture.context)
            let next = try fixture.next(after: task)
            try expectEqual(
                fixture.workspace.reminders(of: next.id, timeZone: .gmt).map(\.trigger), [.beforeDue(seconds: 0)])
        },
        TestCase("given a timed reminder, when the clocks change before the next, then it rings at the same time") {
            let fixture = try Fixture()
            fixture.timeZone = try TimeZone(identifier: "America/Los_Angeles").unwrap("no Los Angeles zone")
            let task = try fixture.repeating("Weigh in", due: "2026-03-06", .weekly(weekdays: [6]))
            // 09:00 in Los Angeles, before summer time starts on 8 March.
            let nine = Date(timeIntervalSince1970: 1_772_816_400)
            try fixture.workspace.addReminder(.at(nine), to: task.id, in: fixture.context)
            try fixture.set(today: "2026-03-06")
            _ = try fixture.workspace.setStatus(.done, of: task.id, in: fixture.context)
            let next = try fixture.next(after: task)
            try expectEqual(next.dueDate, try CalendarDate("2026-03-13"))
            // 09:00 summer time, a week later.
            let nineNextWeek = Date(timeIntervalSince1970: 1_773_417_600)
            let reminders = fixture.workspace.reminders(of: next.id, timeZone: fixture.timeZone)
            try expectEqual(reminders.map(\.trigger), [.at(nineNextWeek)])
        },
        TestCase("given two devices close the same occurrence offline, then both make the same next one") {
            let phone = try Fixture()
            let task = try phone.repeating("Weigh in", due: "2026-11-09", .weekly(weekdays: [2]))
            try phone.set(today: "2026-11-09")
            let mac = Fixture(copying: phone, device: "Mac")
            _ = try phone.workspace.setStatus(.done, of: task.id, in: phone.context)
            _ = try mac.workspace.setStatus(.done, of: task.id, in: mac.context)
            let mine = try phone.next(after: task)
            let theirs = try mac.next(after: task)
            try expectEqual(mine.id, theirs.id)
            let result = RecordValue.merge(base: nil, mine: .task(mine), theirs: .task(theirs))
            try expectEqual(result, .merged(.task(mine)))
        },
        TestCase("given the close is undone, then the next one is gone and the task is open") {
            let fixture = try Fixture()
            let task = try fixture.repeating("Weigh in", due: "2026-11-09", .weekly(weekdays: [2]))
            let change = try fixture.perform { workspace, context throws(LifrError) in
                try workspace.setStatus(.done, of: task.id, in: context)
            }
            let next = try fixture.next(after: task)
            _ = fixture.workspace.undo(change, in: fixture.context)
            try expectEqual(fixture.workspace.tasks[next.id], nil)
            try expectEqual(try fixture.get(task).status, .todo)
        },
        TestCase("given a closed occurrence reopened and closed again, then there's still one next one") {
            let fixture = try Fixture()
            let task = try fixture.repeating("Weigh in", due: "2026-11-09", .weekly(weekdays: [2]))
            _ = try fixture.workspace.setStatus(.done, of: task.id, in: fixture.context)
            _ = try fixture.workspace.setStatus(.todo, of: task.id, in: fixture.context)
            _ = try fixture.workspace.setStatus(.done, of: task.id, in: fixture.context)
            let series = fixture.workspace.tasks.values.filter { $0.recurrence?.seriesID == task.id }
            try expectEqual(series.count, 2)
        },
        TestCase("given a Sub task or a rule that can't happen, then it can't repeat") {
            let fixture = try Fixture()
            let race = try fixture.task("Race")
            let bib = try fixture.task("Pin the bib", parent: race)
            try expectError(.subtaskCannotRepeat(bib.id)) {
                try fixture.workspace.setRecurrence(.daily, of: bib.id, in: fixture.context)
            }
            try expectError(.invalidRecurrence) {
                try fixture.workspace.setRecurrence(.weekly(weekdays: []), of: race.id, in: fixture.context)
            }
        },
        TestCase("given a Project archived with a repeating task, then no next one is made") {
            let fixture = try Fixture()
            let task = try fixture.task("Taper", project: fixture.marathon)
            _ = try fixture.workspace.setRecurrence(.daily, of: task.id, in: fixture.context)
            _ = try fixture.workspace.archive(
                project: fixture.marathon.id, openTasks: .closeAsWontDo, in: fixture.context)
            try expectEqual(fixture.workspace.tasks.values.filter { $0.recurrence != nil }.count, 1)
        },
        TestCase("given a repeating task stops repeating, then closing it makes nothing") {
            let fixture = try Fixture()
            let task = try fixture.repeating("Stretch", due: "2026-11-01", .daily)
            _ = try fixture.workspace.setRecurrence(nil, of: task.id, in: fixture.context)
            _ = try fixture.workspace.setStatus(.done, of: task.id, in: fixture.context)
            try expectEqual(fixture.workspace.tasks.count, 1)
        },
    ]
}
