import Foundation
import LifrCore
import TestingKit

/// Wednesday, Nov 11, 2026.
private func wednesday() throws -> CalendarDate { try CalendarDate("2026-11-11") }

private func read(_ line: String, _ fixture: Fixture, ignoring: Set<String> = [], monthFirst: Bool = true) throws
    -> QuickEntry
{
    QuickEntry(line, today: try wednesday(), in: fixture.workspace, ignoring: ignoring, monthFirst: monthFirst)
}

func quickEntryTests() -> [TestCase] {
    [
        TestCase("given a line with a day, time, priority and label, then each is read and the title is the rest") {
            let fixture = try Fixture()
            let errand = fixture.workspace.createLabel(name: "Errand", in: fixture.context)
            let entry = try read("Call dentist tomorrow 3pm !high #errand", fixture)
            try expectEqual(entry.title, "Call dentist")
            try expectEqual(entry.dueDate, try CalendarDate("2026-11-12"))
            try expectEqual(entry.dueTime, try TimeOfDay("15:00"))
            try expectEqual(entry.priority, .high)
            try expectEqual(entry.labelIDs, [errand.id])
        },
        TestCase("given nothing recognisable, then the whole line is the title") {
            let fixture = try Fixture()
            let entry = try read("Buy 3 apples and a pear", fixture)
            try expectEqual(entry.title, "Buy 3 apples and a pear")
            try expectEqual(entry.parts, [])
            try expectEqual(entry.dueDate, nil)
        },
        TestCase("given days in words, then each is the expected day") {
            let fixture = try Fixture()
            let cases: [(String, String)] = [
                ("Run today", "2026-11-11"),
                ("Run tonight", "2026-11-11"),
                ("Run friday", "2026-11-13"),
                ("Run wednesday", "2026-11-18"),
                ("Run on Monday", "2026-11-16"),
                ("Run next tue", "2026-11-17"),
                ("Run next week", "2026-11-16"),
                ("Run this weekend", "2026-11-14"),
                ("Run next month", "2026-12-01"),
                ("Run in 3 days", "2026-11-14"),
                ("Run in a week", "2026-11-18"),
                ("Run in 2 weeks", "2026-11-25"),
                ("Run Nov 20", "2026-11-20"),
                ("Run 3rd of December", "2026-12-03"),
                ("Run by 5 jan", "2027-01-05"),
                ("Run due 2027-02-01", "2027-02-01"),
                ("Run 12/24", "2026-12-24"),
                ("Run 11/1", "2027-11-01"),
                ("Run 1/2/27", "2027-01-02"),
            ]
            for (line, day) in cases {
                let entry = try read(line, fixture)
                try expectEqual("\(line): \(entry.title)", "\(line): Run")
                try expectEqual("\(line): \(entry.dueDate.map(\.description) ?? "none")", "\(line): \(day)")
            }
        },
        TestCase("given a day-first locale, then 1/2 is the 1st of February") {
            let fixture = try Fixture()
            try expectEqual(try read("Run 1/2", fixture, monthFirst: false).dueDate, try CalendarDate("2027-02-01"))
        },
        TestCase("given times, then each is the expected time, due today without a day") {
            let fixture = try Fixture()
            let cases: [(String, String)] = [
                ("Call at 9am", "09:00"),
                ("Call 12am", "00:00"),
                ("Call noon", "12:00"),
                ("Call 3:30 pm", "15:30"),
                ("Call at 17:45", "17:45"),
                ("Call 12pm", "12:00"),
            ]
            for (line, time) in cases {
                let entry = try read(line, fixture)
                try expectEqual("\(line): \(entry.title)", "\(line): Call")
                try expectEqual("\(line): \(entry.dueTime.map(\.description) ?? "none")", "\(line): \(time)")
                try expectEqual(entry.dueDate, try wednesday())
            }
        },
        TestCase("given numbers and words that aren't days or times, then they stay in the title") {
            let fixture = try Fixture()
            for line in ["Read chapter 3", "Mow the lawn sat", "Plan may party", "Fix 25:00 bug", "Wow !", "Run 13pm"] {
                try expectEqual(try read(line, fixture).title, line)
            }
        },
        TestCase("given an ignored word, then it stays in the title and the next one is read") {
            let fixture = try Fixture()
            let plain = try read("Read Tomorrow and Tomorrow", fixture)
            try expectEqual(plain.title, "Read and Tomorrow")
            let ignored = try read("Read Tomorrow and Tomorrow", fixture, ignoring: ["tomorrow"])
            try expectEqual(ignored.title, "Read Tomorrow and Tomorrow")
            try expectEqual(ignored.dueDate, nil)
        },
        TestCase("given a part, then its range covers its words in the line") {
            let fixture = try Fixture()
            let entry = try read("Call mom on friday, then rest", fixture)
            try expectEqual(entry.parts.map(\.range), [9..<19])
            try expectEqual(entry.parts.map(\.key), ["on friday"])
        },
        TestCase("given an Avenue or a Project by the start of its name, then the task goes there") {
            let fixture = try Fixture()
            let avenue = try read("Stretch @heal", fixture)
            try expectEqual(avenue.avenueID, fixture.health.id)
            try expectEqual(avenue.projectID, nil)
            let project = try read("Long run /half", fixture)
            try expectEqual(project.title, "Long run")
            try expectEqual(project.avenueID, fixture.health.id)
            try expectEqual(project.projectID, fixture.marathon.id)
        },
        TestCase("given a name that starts two labels, then neither is picked") {
            let fixture = try Fixture()
            _ = fixture.workspace.createLabel(name: "Errand", in: fixture.context)
            _ = fixture.workspace.createLabel(name: "Errands abroad", in: fixture.context)
            let entry = try read("Post #err", fixture)
            try expectEqual(entry.title, "Post #err")
            try expectEqual(entry.labelIDs, [])
        },
        TestCase("given a repeat, then the task repeats and is due the first day it falls on") {
            let fixture = try Fixture()
            let cases: [(String, String, Recurrence.Rule, Int, String)] = [
                ("Weigh in every Monday", "Weigh in", .weekly(weekdays: [2]), 1, "2026-11-16"),
                ("Stretch daily", "Stretch", .daily, 1, "2026-11-11"),
                ("Water plants every other week", "Water plants", .weekly(weekdays: [4]), 2, "2026-11-11"),
                ("Pay rent every month on the 1st", "Pay rent", .monthly(day: 1), 1, "2026-12-01"),
                ("Run every 3 days at 7am", "Run", .daily, 3, "2026-11-11"),
                ("Stand up every weekday", "Stand up", .weekly(weekdays: [2, 3, 4, 5, 6]), 1, "2026-11-11"),
            ]
            for (line, title, rule, interval, due) in cases {
                let entry = try read(line, fixture)
                try expectEqual(entry.title, title)
                try expectEqual(entry.repeats?.rule, rule)
                try expectEqual(entry.repeats?.interval, interval)
                try expectEqual(entry.dueDate, try CalendarDate(due))
            }
        },
        TestCase("given a repeat and a day, then it's due that day; every alone is a word") {
            let fixture = try Fixture()
            let dated = try read("Weigh in every Monday on Nov 23", fixture)
            try expectEqual(dated.dueDate, try CalendarDate("2026-11-23"))
            let plain = try read("Read every page", fixture)
            try expectEqual(plain.title, "Read every page")
            try expectEqual(plain.parts, [])
        },
        TestCase("given tomorrow typed at 23:59 and at 00:01, then it's the day after the clock's day") {
            let fixture = try Fixture()
            let zone = try TimeZone(identifier: "America/New_York").unwrap()
            // 23:59 on Nov 11 in New York, then 00:01 on Nov 12.
            let late = try ISO8601DateFormatter().date(from: "2026-11-12T04:59:00Z").unwrap()
            let early = late.addingTimeInterval(120)
            for (now, day) in [(late, "2026-11-12"), (early, "2026-11-13")] {
                let today = CalendarDate(now, in: zone)
                let entry = QuickEntry("Call mum tomorrow", today: today, in: fixture.workspace)
                try expectEqual(entry.title, "Call mum")
                try expectEqual(entry.dueDate, try CalendarDate(day))
            }
        },
    ]
}
