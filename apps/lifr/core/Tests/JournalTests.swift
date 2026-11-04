import Foundation
import LifrCore
import LifrTesting

extension Fixture {
    /// Runs `operation` the way the stores do, so it gets a journal entry.
    @discardableResult
    func perform(_ operation: (inout Workspace, Context) throws(LifrError) -> Change) throws -> Change {
        try workspace.perform(in: context, operation)
    }

    func revert(_ entries: [UUID?]) throws -> Change {
        try perform { workspace, context throws(LifrError) in
            try workspace.revert(entries.compactMap(\.self), in: context)
        }
    }

    func entry(_ change: Change) -> JournalEntry? {
        workspace.journal.first { $0.id == change.entry }
    }
}

func journalTests() -> [TestCase] {
    [
        TestCase("given an edit, when it's performed, then one entry has who made it and what changed") {
            let fixture = try Fixture()
            fixture.actor = .user(device: "Mac", surface: .mac)
            let task = try fixture.task("Run")
            let before = fixture.workspace.journal.count
            let change = try fixture.perform { workspace, context throws(LifrError) in
                try workspace.edit(task.id, .init(title: "Run 5 km", priority: .high), in: context)
            }
            try expectEqual(fixture.workspace.journal.count, before + 1)
            let entry = try fixture.entry(change).unwrap()
            try expectEqual(entry.actor.surface, .mac)
            try expectEqual(entry.at, fixture.now)
            try expectEqual(entry.steps.map(\.record), [RecordID(.task, task.id)])
            try expectEqual(
                entry.steps[0].fields,
                [
                    FieldChange(field: .title, before: .text("Run"), after: .text("Run 5 km")),
                    FieldChange(field: .priority, before: .priority(.none), after: .priority(.high)),
                ])
        },
        TestCase("given a change that names its actor, when it's performed, then the entry credits that actor") {
            let fixture = try Fixture()
            let task = try fixture.task("Run")
            let change = try fixture.perform { workspace, context throws(LifrError) in
                var change = try workspace.edit(task.id, .init(title: "Run 5 km"), in: context)
                change.actor = .assistant(device: "iPhone")
                return change
            }
            try expectEqual(fixture.entry(change)?.actor, .assistant(device: "iPhone"))
        },
        TestCase("given an operation that fails, when it's performed, then nothing changes and no entry is written") {
            let fixture = try Fixture()
            let task = try fixture.task("Run")
            let before = fixture.workspace
            try expectError(.invalidTitle(length: 0)) {
                try fixture.perform { workspace, context throws(LifrError) in
                    _ = try workspace.edit(task.id, .init(priority: .high), in: context)
                    return try workspace.edit(task.id, .init(title: ""), in: context)
                }
            }
            try expectEqual(fixture.workspace, before)
        },
        TestCase("given an operation that changes nothing, when it's performed, then no entry is written") {
            let fixture = try Fixture()
            let change = try fixture.perform { _, _ in Change() }
            try expectEqual(change.entry, nil)
            try expectEqual(fixture.workspace.journal, [])
        },
        TestCase("given an edit, when it's reverted, then the fields are back and the revert is an entry too") {
            let fixture = try Fixture()
            let task = try fixture.task("Run")
            let edit = try fixture.perform { workspace, context throws(LifrError) in
                try workspace.edit(task.id, .init(title: "Run 5 km", priority: .high), in: context)
            }
            let revert = try fixture.revert([edit.entry])
            try expectEqual(fixture.get(task).title, "Run")
            try expectEqual(fixture.get(task).priority, .none)
            try expectEqual(fixture.entry(revert)?.reverts, [try edit.entry.unwrap()])
            try expectEqual(revert.events.map(\.reason), [.undo, .undo])

            _ = try fixture.revert([revert.entry])
            try expectEqual(fixture.get(task).title, "Run 5 km")
            try expectEqual(fixture.get(task).priority, .high)
        },
        TestCase("given a field changed since, when the earlier entry is reverted, then only that field is kept") {
            let fixture = try Fixture()
            let task = try fixture.task("Run")
            let first = try fixture.perform { workspace, context throws(LifrError) in
                try workspace.edit(task.id, .init(title: "Run 5 km", priority: .high), in: context)
            }
            try fixture.perform { workspace, context throws(LifrError) in
                try workspace.edit(task.id, .init(title: "Run 10 km"), in: context)
            }
            let revert = try fixture.revert([first.entry])
            try expectEqual(fixture.get(task).title, "Run 10 km")
            try expectEqual(fixture.get(task).priority, .none)
            let skipped = SkippedRevert(record: RecordID(.task, task.id), field: .title, reason: .changedSince)
            try expectEqual(revert.skipped, [skipped])
        },
        TestCase("given a task that was added, when that's reverted, then the task is in the trash") {
            let fixture = try Fixture()
            let change = try fixture.perform { workspace, context throws(LifrError) in
                try workspace.createTask(.init(title: "Stretch", avenueID: fixture.health.id), in: context)
            }
            let id = try fixture.entry(change).unwrap().steps[0].record.id
            _ = try fixture.revert([change.entry])
            try expect(fixture.workspace.tasks[id]?.deletedAt != nil, "the task should be in the trash")
        },
        TestCase("given a label that was added, when that's reverted, then it's removed unless a task has it") {
            let fixture = try Fixture()
            let task = try fixture.task("Run")
            let unused = try fixture.perform { workspace, context in
                workspace.createLabel(name: "outdoors", in: context)
                return Change()
            }
            var used: UUID?
            let usedChange = try fixture.perform { workspace, context in
                let label = workspace.createLabel(name: "morning", in: context)
                used = label.id
                return Change()
            }
            let label = try used.unwrap()
            try fixture.perform { workspace, context throws(LifrError) in
                try workspace.edit(task.id, .init(labelIDs: [label]), in: context)
            }
            _ = try fixture.revert([unused.entry])
            try expectEqual(fixture.workspace.labels.values.map(\.name), ["morning"])
            let revert = try fixture.revert([usedChange.entry])
            try expectEqual(revert.skipped.map(\.reason), [.inUse])
            try expectEqual(fixture.workspace.labels.count, 1)
        },
        TestCase("given a settings change, when it's reverted, then the old setting is back") {
            let fixture = try Fixture()
            let change = try fixture.perform { workspace, _ in
                workspace.settings.procrastinatedAfterDays = 3
                return Change()
            }
            try expectEqual(fixture.entry(change)?.steps.map(\.record), [.settings])
            _ = try fixture.revert([change.entry])
            try expectEqual(fixture.workspace.settings.procrastinatedAfterDays, 7)
        },
        TestCase("given changes in and out of a conversation, when the conversation is reverted, then only its") {
            let fixture = try Fixture()
            let task = try fixture.task("Run")
            let conversation = UUID()
            for (title, inConversation) in [("Run 5 km", true), ("Run 5 km slowly", false)] {
                try fixture.perform { workspace, context throws(LifrError) in
                    var change = try workspace.edit(task.id, .init(title: title), in: context)
                    if inConversation { change.conversation = conversation }
                    return change
                }
            }
            try fixture.perform { workspace, context throws(LifrError) in
                var change = try workspace.edit(task.id, .init(priority: .urgent), in: context)
                change.conversation = conversation
                return change
            }
            let entries = fixture.workspace.entries(inConversation: conversation)
            try expectEqual(entries.count, 2)
            let revert = try fixture.revert(entries.map(\.id))
            try expectEqual(fixture.get(task).priority, .none)
            try expectEqual(fixture.get(task).title, "Run 5 km slowly")
            try expectEqual(revert.skipped.map(\.field), [.title])
        },
        TestCase("given changes over two days, when everything since the second is reverted, then the first stays") {
            let fixture = try Fixture()
            let task = try fixture.task("Run")
            try fixture.perform { workspace, context throws(LifrError) in
                try workspace.edit(task.id, .init(title: "Run 5 km"), in: context)
            }
            fixture.advance(days: 1)
            let since = fixture.now
            for priority in [Priority.low, .high] {
                try fixture.perform { workspace, context throws(LifrError) in
                    try workspace.edit(task.id, .init(priority: priority), in: context)
                }
            }
            let revert = try fixture.revert(fixture.workspace.entries(since: since).map(\.id))
            try expectEqual(fixture.get(task).priority, .none)
            try expectEqual(fixture.get(task).title, "Run 5 km")
            try expectEqual(revert.skipped, [])
        },
        TestCase("given an unknown entry, when it's reverted, then notFound and nothing changes") {
            let fixture = try Fixture()
            let before = fixture.workspace
            let id = UUID()
            try expectError(.notFound(.change, id.uuidString)) { try fixture.revert([id]) }
            try expectEqual(fixture.workspace, before)
        },
    ]
}

extension Optional {
    fileprivate func unwrap(file: StaticString = #filePath, line: UInt = #line) throws -> Wrapped {
        guard let self else {
            try expect(false, "expected a value", file: file, line: line)
            preconditionFailure("expect(false) throws")
        }
        return self
    }
}
