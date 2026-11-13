import Foundation
import LifrCore
import TestingKit

/// A task as both devices last agreed on it, to change on each side.
private func base() throws -> (Fixture, LifrTask) {
    let fixture = try Fixture()
    let task = try fixture.task("Buy socks")
    return (fixture, task)
}

private func merge(_ base: LifrTask?, _ mine: LifrTask?, _ theirs: LifrTask?) -> MergeResult {
    RecordValue.merge(
        base: base.map(RecordValue.task), mine: mine.map(RecordValue.task), theirs: theirs.map(RecordValue.task))
}

private func merged(_ result: MergeResult) throws -> LifrTask? {
    guard case .merged(let value) = result else { throw Unexpected("expected a merge, got \(result)") }
    guard let value else { return nil }
    guard case .task(let task) = value else { throw Unexpected("expected a task, got \(value)") }
    return task
}

private func conflict(_ result: MergeResult) throws -> SyncConflict {
    guard case .conflict(let conflict) = result else { throw Unexpected("expected a conflict, got \(result)") }
    return conflict
}

private func task(_ value: RecordValue?) throws -> LifrTask {
    guard case .task(let task)? = value else { throw Unexpected("expected a task, got \(String(describing: value))") }
    return task
}

func mergeTests() -> [TestCase] {
    [
        TestCase("given only the other device changed a task, then its version is taken") {
            let (_, original) = try base()
            var theirs = original
            theirs.title = "Buy wool socks"
            try expectEqual(merged(merge(original, original, theirs)), theirs)
        },
        TestCase("given only this device changed a task, then this version stays") {
            let (_, original) = try base()
            var mine = original
            mine.priority = .high
            try expectEqual(merged(merge(original, mine, original)), mine)
        },
        TestCase("given each device changed a different field, then both changes are kept") {
            let (_, original) = try base()
            var mine = original
            mine.priority = .high
            var theirs = original
            theirs.title = "Buy wool socks"
            theirs.updatedAt += 60
            let result = try merged(merge(original, mine, theirs)).unwrap()
            try expectEqual(result.priority, .high)
            try expectEqual(result.title, "Buy wool socks")
            try expectEqual(result.updatedAt, theirs.updatedAt)
        },
        TestCase("given both devices made the same change, then it merges") {
            let (_, original) = try base()
            var mine = original
            mine.title = "Buy wool socks"
            var theirs = mine
            theirs.priority = .low
            try expectEqual(merged(merge(original, mine, theirs))?.priority, .low)
        },
        TestCase("given both devices changed the title differently, then the title conflicts and the rest merges") {
            let (_, original) = try base()
            var mine = original
            mine.title = "Buy wool socks"
            var theirs = original
            theirs.title = "Buy cotton socks"
            theirs.priority = .high
            let found = try conflict(merge(original, mine, theirs))
            try expectEqual(found.kind, .fields([.title]))
            try expectEqual(task(found.merged).title, "Buy wool socks")
            try expectEqual(task(found.merged).priority, .high)
            try expectEqual(task(found.resolved([.title: .theirs])).title, "Buy cotton socks")
            try expectEqual(task(found.resolved([:])).title, "Buy wool socks")
            try expectEqual(task(found.resolved([.title: .theirs])).priority, .high)
        },
        TestCase("given one device moved a task and the other changed its parent, then placement conflicts as one") {
            let (fixture, original) = try base()
            let parent = try fixture.task("Errands")
            var mine = original
            mine.projectID = fixture.marathon.id
            var theirs = original
            theirs.parentID = parent.id
            let found = try conflict(merge(original, mine, theirs))
            try expectEqual(found.kind, .fields([.avenue]))
            let chosen = try task(found.resolved([.avenue: .theirs]))
            try expectEqual(chosen.parentID, parent.id)
            try expectEqual(chosen.projectID, nil)
        },
        TestCase("given one device trashed a task the other edited, then the whole task conflicts") {
            let (fixture, original) = try base()
            var mine = original
            mine.deletedAt = fixture.now
            var theirs = original
            theirs.title = "Buy wool socks"
            let found = try conflict(merge(original, mine, theirs))
            try expectEqual(found.kind, .removed(by: .mine))
            try expectEqual(task(found.resolved(keeping: .theirs)).deletedAt, nil)
            try expect(task(found.resolved(keeping: .mine)).deletedAt != nil)
        },
        TestCase("given one device trashed a task the other didn't change, then it's trashed") {
            let (fixture, original) = try base()
            var theirs = original
            theirs.deletedAt = fixture.now
            try expectEqual(merged(merge(original, original, theirs))?.deletedAt, fixture.now)
        },
        TestCase("given both devices trashed a task at different times, then it merges") {
            let (fixture, original) = try base()
            var mine = original
            mine.deletedAt = fixture.now
            var theirs = original
            theirs.deletedAt = fixture.now + 60
            try expect(merged(merge(original, mine, theirs))?.deletedAt != nil)
        },
        TestCase("given the other device removed a task this one edited, then the whole task conflicts") {
            let (_, original) = try base()
            var mine = original
            mine.title = "Buy wool socks"
            let found = try conflict(merge(original, mine, nil))
            try expectEqual(found.kind, .removed(by: .theirs))
            try expectEqual(found.resolved(keeping: .theirs), nil)
            try expectEqual(task(found.resolved(keeping: .mine)).title, "Buy wool socks")
        },
        TestCase("given the other device removed a task this one didn't change, then it's removed") {
            let (_, original) = try base()
            try expectEqual(merged(merge(original, original, nil)), nil)
        },
        TestCase("given a task new on the other device, then it's added") {
            let (_, original) = try base()
            try expectEqual(merged(merge(nil, nil, original)), original)
        },
        TestCase("given an import, then records, entries and events are added without an entry of its own") {
            let phone = try Fixture()
            let mac = Fixture(copying: phone, device: "Mac")
            let task = try mac.task("Made on the Mac")
            try mac.perform { workspace, context throws(LifrError) in
                try workspace.edit(task.id, .init(priority: .high), in: context)
            }
            let edited = try mac.get(task)
            let before = phone.workspace.journal.count
            phone.workspace.importSynced(
                records: [RecordID(.task, task.id): .task(edited)], journal: mac.workspace.journal,
                history: mac.workspace.history)
            try expectEqual(phone.get(task), edited)
            try expectEqual(phone.workspace.journal.count, before + 1)
            try expectEqual(Set(phone.workspace.history.map(\.id)), Set(mac.workspace.history.map(\.id)))
            phone.workspace.importSynced(journal: mac.workspace.journal, history: mac.workspace.history)
            try expectEqual(phone.workspace.journal.count, before + 1)
        },
        TestCase("given a task edited here while sync merged it, when the import lands, then both changes stay") {
            let phone = try Fixture()
            let task = try phone.task("Buy socks")
            let read = try phone.get(task)
            var theirs = read
            theirs.priority = .high
            try phone.perform { workspace, context throws(LifrError) in
                try workspace.edit(task.id, .init(title: "Buy wool socks"), in: context)
            }
            let id = RecordID(.task, task.id)
            phone.workspace.importSynced(records: [id: .task(theirs)], seen: [id: .task(read)])
            try expectEqual(phone.get(task).title, "Buy wool socks")
            try expectEqual(phone.get(task).priority, .high)
        },
        TestCase("given an import that removes a record, then it's gone") {
            let fixture = try Fixture()
            let task = try fixture.task("Gone elsewhere")
            fixture.workspace.importSynced(records: [RecordID(.task, task.id): RecordValue?.none])
            try expectEqual(fixture.workspace.tasks[task.id], nil)
        },
        TestCase("given resolved choices performed, then they're one entry that can be reverted") {
            let fixture = try Fixture()
            let original = try fixture.task("Buy socks")
            var chosen = original
            chosen.title = "Buy wool socks"
            let change = try fixture.perform { workspace, _ in
                workspace.resolveSynced([RecordID(.task, original.id): .task(chosen)])
            }
            try expectEqual(fixture.get(original).title, "Buy wool socks")
            _ = try fixture.revert([change.entry])
            try expectEqual(fixture.get(original).title, "Buy socks")
        },
    ]
}

struct Unexpected: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}
