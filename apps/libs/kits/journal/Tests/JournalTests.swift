import Foundation
import JournalKit
import TestingKit

private func note(_ item: Item?) throws -> Note {
    guard case .note(let note)? = item else { throw Unexpected("expected a note, got \(String(describing: item))") }
    return note
}

func journalTests() -> [TestCase] {
    [
        TestCase("given a renamed note, then its step lists only the title") {
            var renamed = shopping
            renamed.title = "Groceries"
            renamed.editedAt = at(60)
            let step = Step(before: .note(shopping), after: .note(renamed))
            try expectEqual(
                step.fields, [FieldChange(field: .title, before: .text("Shopping"), after: .text("Groceries"))])
        },
        TestCase("given two versions of the notes, then there's a step per note that differs") {
            var renamed = shopping
            renamed.title = "Groceries"
            let old = [shopping.id: shopping, ideas.id: ideas]
            let new = [shopping.id: renamed, work.id: Note(id: work.id, title: "New")]
            let steps = Step.steps(from: old, to: new, as: Item.note)
            try expectEqual(steps.map(\.record.id), [shopping.id, ideas.id, work.id])
            try expectEqual(steps[1].after, nil)
            try expectEqual(steps[2].before, nil)
        },
        TestCase("given an entry saved as JSON, then it reads back the same") {
            let saved = entry(1, at: 10, conversation: UUID())
            let data = try JSONEncoder().encode(saved)
            try expectEqual(try JSONDecoder().decode(Entry.self, from: data), saved)
        },
        TestCase("given a saved step with neither side, then reading it fails") {
            let data = Data("{}".utf8)
            try expect((try? JSONDecoder().decode(Step.self, from: data)) == nil)
        },
        TestCase("given an entry reverted and that revert reverted, then only the first revert is undone") {
            let first = entry(1, at: 10)
            let undo = entry(2, at: 20, reverts: [first.id])
            let redo = entry(3, at: 30, reverts: [undo.id])
            try expectEqual([first, undo, redo].undone(), [undo.id])
        },
        TestCase("given entries over time, then those since a time are listed newest first") {
            let entries = [entry(1, at: 10), entry(2, at: 20), entry(3, at: 30)]
            try expectEqual(entries.made(since: at(20)).map(\.at), [at(30), at(20)])
        },
        TestCase("given entries in two conversations, then each conversation lists its own") {
            let chat = UUID()
            let entries = [
                entry(1, at: 10, conversation: chat), entry(2, at: 20), entry(3, at: 30, conversation: chat),
            ]
            try expectEqual(entries.made(inConversation: chat).map(\.at), [at(30), at(10)])
        },
        TestCase("given entries from another device, then new ones are added in time order") {
            var journal = [entry(1, at: 10), entry(3, at: 30)]
            journal.add(synced: [entry(2, at: 20), entry(3, at: 30)])
            try expectEqual(journal.map(\.at), [at(10), at(20), at(30)])
        },
    ]
}

func revertTests() -> [TestCase] {
    [
        TestCase("given a rename, when it's reverted, then the old title is back") {
            var renamed = shopping
            renamed.title = "Groceries"
            let step = Step(before: .note(shopping), after: .note(renamed))
            try expectEqual(step.revert(current: .note(renamed)), .update(.note(shopping), skipped: []))
        },
        TestCase("given a field changed again since, when the entry is reverted, then that field is skipped") {
            var renamed = shopping
            renamed.title = "Groceries"
            renamed.body = "Milk"
            let step = Step(before: .note(shopping), after: .note(renamed))
            var later = renamed
            later.title = "Weekly shop"
            guard case .update(let value, let skipped) = step.revert(current: .note(later)) else {
                throw Unexpected("expected an update")
            }
            try expectEqual(skipped, [.title])
            try expectEqual(try note(value).title, "Weekly shop")
            try expectEqual(try note(value).body, "")
        },
        TestCase("given every changed field changed again, when reverted, then nothing changes") {
            var renamed = shopping
            renamed.title = "Groceries"
            let step = Step(before: .note(shopping), after: .note(renamed))
            var later = renamed
            later.title = "Weekly shop"
            try expectEqual(step.revert(current: .note(later)), .update(nil, skipped: [.title]))
        },
        TestCase("given an added note, when reverted, then it's to be removed") {
            let step = Step(before: nil, after: .note(shopping))
            try expectEqual(step.revert(current: .note(shopping)), .remove(.note(shopping)))
            try expectEqual(step.revert(current: nil), .nothing)
        },
        TestCase("given a removed note, when reverted, then it's put back") {
            let step = Step(before: .note(shopping), after: nil)
            try expectEqual(step.revert(current: nil), .restore(.note(shopping)))
        },
        TestCase("given a changed note that's gone since, when reverted, then it can't be") {
            var renamed = shopping
            renamed.title = "Groceries"
            let step = Step(before: .note(shopping), after: .note(renamed))
            try expectEqual(step.revert(current: nil), .gone)
        },
    ]
}
