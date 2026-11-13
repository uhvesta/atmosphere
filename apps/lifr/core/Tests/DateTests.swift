import Foundation
import LifrCore
import TestingKit

func dateTests() -> [TestCase] {
    [
        TestCase("given YYYY-MM-DD, then it parses and prints the same") {
            try expectEqual(try CalendarDate("2026-11-05").description, "2026-11-05")
        },
        TestCase("given a day that doesn't exist, then it's rejected") {
            try expectError(.invalidDate("2026-02-29")) { try CalendarDate("2026-02-29") }
            try expectError(.invalidDate("2026-11-5")) { try CalendarDate("2026-11-5") }
            try expectEqual(try CalendarDate("2028-02-29").day, 29)
        },
        TestCase("given an instant, then its calendar day depends on the time zone") {
            let instant = Date(timeIntervalSince1970: 1_793_664_000)  // 2026-11-03T00:00Z
            try expectEqual(CalendarDate(instant, in: TimeZone.gmt), try CalendarDate("2026-11-03"))
            try expectEqual(
                CalendarDate(instant, in: TimeZone(identifier: "America/Los_Angeles").unwrap()),
                try CalendarDate("2026-11-02"))
        },
        TestCase("given instants across years and time zones, then the day and time are the calendar's") {
            let zones = try ["UTC", "America/Los_Angeles", "Asia/Kolkata", "Pacific/Chatham", "Pacific/Kiritimati"]
                .map { try TimeZone(identifier: $0).unwrap() }
            // From 1930 to 2090, every 37 hours and 13 minutes, which lands on
            // every hour of the day and both sides of each daylight saving change.
            for instant in stride(from: -1_262_304_000.0, to: 3_786_825_600, by: 133_980) {
                let date = Date(timeIntervalSince1970: instant)
                for zone in zones {
                    var calendar = Calendar(identifier: .gregorian)
                    calendar.timeZone = zone
                    let parts = calendar.dateComponents([.year, .month, .day, .hour, .minute], from: date)
                    let day = CalendarDate(date, in: zone)
                    let time = TimeOfDay(date, in: zone)
                    let expected = [parts.year, parts.month, parts.day, parts.hour, parts.minute].map { $0 ?? -1 }
                    try expectEqual([day.year, day.month, day.day, time.hour, time.minute], expected)
                }
            }
        },
        TestCase("given a time, then it must be 24-hour HH:MM") {
            try expectEqual(try TimeOfDay("09:30").description, "09:30")
            try expectError(.invalidTime("24:00")) { try TimeOfDay("24:00") }
        },
        TestCase("given an open task due yesterday, or today at an earlier time, then it's overdue") {
            let fixture = try Fixture()
            var task = try fixture.task("Book physio")
            let today = try CalendarDate("2026-11-03")
            let noon = try TimeOfDay("12:00")
            task.dueDate = try CalendarDate("2026-11-02")
            try expect(task.isOverdue(today: today, time: noon))
            task.dueDate = today
            try expect(!task.isOverdue(today: today, time: noon))
            task.dueTime = try TimeOfDay("09:00")
            try expect(task.isOverdue(today: today, time: noon))
            task.status = .done
            try expect(!task.isOverdue(today: today, time: noon))
        },
        TestCase("given a due time without a due date, then it's refused") {
            let fixture = try Fixture()
            let task = try fixture.task("Book physio")
            try expectError(.dueTimeWithoutDueDate) {
                try fixture.workspace.edit(task.id, .init(dueTime: .some(try TimeOfDay("09:00"))), in: fixture.context)
            }
        },
        TestCase("given tasks of mixed priority and due dates, then lists sort by priority then due date") {
            let fixture = try Fixture()
            var low = try fixture.task("Low")
            low.priority = .low
            var urgent = try fixture.task("Urgent")
            urgent.priority = .urgent
            var soon = try fixture.task("Soon")
            soon.dueDate = try CalendarDate("2026-11-04")
            let undated = try fixture.task("Undated")
            let sorted = [undated, low, soon, urgent].sorted(by: LifrTask.listOrder).map(\.title)
            try expectEqual(sorted, ["Urgent", "Low", "Soon", "Undated"])
        },
        TestCase("given many tasks, then the listed ones come in list order") {
            let fixture = try Fixture()
            let priorities: [Priority] = [.none, .low, .high, .urgent]
            for index in 0..<40 {
                let task = try fixture.task("Task \(index)")
                let due = index % 3 == 0 ? nil : try CalendarDate("2026-11-0\(index % 9 + 1)")
                _ = try fixture.workspace.edit(
                    task.id, .init(priority: priorities[index % 4], dueDate: .some(due)), in: fixture.context)
            }
            let tasks = fixture.workspace.listedTasks
            try expectEqual(tasks.count, 40)
            try expectEqual(tasks.map(\.id), tasks.sorted(by: LifrTask.listOrder).map(\.id))
        },
    ]
}
