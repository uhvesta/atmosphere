import Foundation
import JournalKit
import TestingKit

typealias Archived = ArchivedEntry<Item, String>

func archiveTests() -> [TestCase] {
    [
        TestCase("given a renamed note, then its archived step keeps only the title, before and after") {
            var renamed = shopping
            renamed.title = "Groceries"
            renamed.body = ""
            let entry = Entry(
                id: UUID(), actor: "Sam", at: at(10), steps: [Step(before: .note(shopping), after: .note(renamed))])
            try expectEqual(
                entry.archived.steps,
                [
                    ArchivedStep(
                        record: RecordID(.note, shopping.id), change: .changed,
                        fields: [FieldChange(field: .title, before: .text("Shopping"), after: .text("Groceries"))])
                ])
        },
        TestCase("given an added and a removed note, then their archived steps say so with every field") {
            let entry = Entry(
                id: UUID(), actor: "Sam", at: at(10),
                steps: [Step(before: nil, after: .note(shopping)), Step(before: .note(ideas), after: nil)])
            try expectEqual(entry.archived.steps.map(\.change), [.added, .removed])
            try expectEqual(entry.archived.steps[0].fields.map(\.field), [.title, .position])
        },
        TestCase("given entries either side of a cutoff, then only the older ones are archived, by month") {
            let june = Date(timeIntervalSince1970: 1_782_864_000)  // 2026-07-01 00:00 UTC, less a day below.
            let entries = [
                entry(1, at: june.timeIntervalSince1970 - 86_400), entry(2, at: june.timeIntervalSince1970),
                entry(3, at: june.timeIntervalSince1970 + 86_400 * 40),
            ]
            let (kept, archived) = entries.archiving(before: at(june.timeIntervalSince1970 + 86_400))
            try expectEqual(kept.map(\.id), [entries[2].id])
            try expectEqual(archived[JournalMonth(year: 2026, month: 6)]?.map(\.id), [entries[0].id])
            try expectEqual(archived[JournalMonth(year: 2026, month: 7)]?.map(\.id), [entries[1].id])
        },
        TestCase("given an entry two devices archived, then merging keeps it once, oldest first") {
            let first = entry(1, at: 10).archived
            let second = entry(2, at: 20).archived
            try expectEqual([second, first].merging([first]).map(\.id), [first.id, second.id])
        },
        TestCase("given an archived entry saved as JSON, then it reads back the same") {
            let saved = entry(1, at: 10, reverts: [UUID()]).archived
            let data = try JSONEncoder().encode([saved])
            try expectEqual(try JSONDecoder().decode([Archived].self, from: data), [saved])
        },
        TestCase("given a month written out, then it reads back, and other text doesn't") {
            try expectEqual(JournalMonth("2026-07"), JournalMonth(year: 2026, month: 7))
            try expectEqual(
                JournalMonth(JournalMonth(year: 987, month: 3).description), JournalMonth(year: 987, month: 3))
            try expectEqual(JournalMonth("2026-13"), nil)
            try expectEqual(JournalMonth("July"), nil)
        },
    ]
}
