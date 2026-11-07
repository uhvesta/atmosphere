import Foundation
import JournalKit
import TestingKit

private func merge(_ base: Note?, _ mine: Note?, _ theirs: Note?) -> MergeResult<Item> {
    Item.merge(base: base.map(Item.note), mine: mine.map(Item.note), theirs: theirs.map(Item.note))
}

private func merged(_ result: MergeResult<Item>) throws -> Note? {
    guard case .merged(let value) = result else { throw Unexpected("expected a merge, got \(result)") }
    guard let value else { return nil }
    guard case .note(let note) = value else { throw Unexpected("expected a note, got \(value)") }
    return note
}

private func conflict(_ result: MergeResult<Item>) throws -> SyncConflict<Item> {
    guard case .conflict(let conflict) = result else { throw Unexpected("expected a conflict, got \(result)") }
    return conflict
}

func mergeTests() -> [TestCase] {
    [
        TestCase("given only the other device changed a note, then its version is taken") {
            var theirs = shopping
            theirs.title = "Groceries"
            try expectEqual(merged(merge(shopping, shopping, theirs)), theirs)
        },
        TestCase("given each device changed a different field, then both changes are kept") {
            var mine = shopping
            mine.body = "Milk"
            var theirs = shopping
            theirs.title = "Groceries"
            theirs.editedAt = at(60)
            let result = try merged(merge(shopping, mine, theirs)).unwrap()
            try expectEqual(result.body, "Milk")
            try expectEqual(result.title, "Groceries")
            try expectEqual(result.editedAt, at(60))
        },
        TestCase("given both devices changed the title differently, then it's a conflict about the title") {
            var mine = shopping
            mine.title = "Groceries"
            var theirs = shopping
            theirs.title = "Weekly shop"
            let found = try conflict(merge(shopping, mine, theirs))
            try expectEqual(found.kind, .fields([.title]))
            try expectEqual(try merged(.merged(found.resolved([.title: .theirs]))), theirs)
            try expectEqual(try merged(.merged(found.resolved([:]))), mine)
        },
        TestCase("given one device moved a note and the other reordered it, then the placement conflicts as one") {
            var mine = shopping
            mine.folder = home.id
            var theirs = shopping
            theirs.position = 3
            let found = try conflict(merge(shopping, mine, theirs))
            try expectEqual(found.kind, .fields([.folder]))
            let resolved = try merged(.merged(found.resolved([.folder: .theirs]))).unwrap()
            try expectEqual(resolved.folder, nil)
            try expectEqual(resolved.position, 3)
            try expectEqual(found.sides(of: .folder).map(\.field), [.folder, .position])
        },
        TestCase("given one device trashed a note the other edited, then it's a conflict about the whole note") {
            var mine = shopping
            mine.deletedAt = at(5)
            var theirs = shopping
            theirs.body = "Milk"
            let found = try conflict(merge(shopping, mine, theirs))
            try expectEqual(found.kind, .removed(by: .mine))
            try expectEqual(try merged(.merged(found.resolved([.deletedAt: .theirs]))), theirs)
        },
        TestCase("given both devices trashed a note at different times, then it merges") {
            var mine = shopping
            mine.deletedAt = at(5)
            var theirs = shopping
            theirs.deletedAt = at(9)
            try expectEqual(try merged(merge(shopping, mine, theirs)), mine)
        },
        TestCase("given one device removed a note the other edited, then it's a conflict") {
            var theirs = shopping
            theirs.title = "Groceries"
            let found = try conflict(merge(shopping, nil, theirs))
            try expectEqual(found.kind, .removed(by: .mine))
            try expectEqual(found.resolved(keeping: .theirs), .note(theirs))
        },
        TestCase("given a conflict saved as JSON, then it reads back the same") {
            var mine = shopping
            mine.title = "Groceries"
            var theirs = shopping
            theirs.title = "Weekly shop"
            let found = try conflict(merge(shopping, mine, theirs))
            let data = try JSONEncoder().encode(found)
            try expectEqual(try JSONDecoder().decode(SyncConflict<Item>.self, from: data), found)
        },
    ]
}
