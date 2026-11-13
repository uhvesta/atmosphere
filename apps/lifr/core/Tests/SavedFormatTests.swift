import Foundation
import LifrCore
import TestingKit

// JSON that Lifr wrote before the journal moved to JournalKit, as saved on
// devices and in iCloud. Each must read back, and write back the same, so
// devices on either version understand each other.

private let savedEntry = #"""
    {"actor":{"device":"Mac","kind":"user","surface":"mac"},"at":-978306200,"conversation":"01000000-000\#
    0-0000-0000-000000000001","id":"02000000-0000-0000-0000-000000000002","reverts":["01000000-0000-0000\#
    -0000-000000000001"],"steps":[{"after":{"label":{"_0":{"color":"blue","createdAt":-978306200,"id":"0\#
    1000000-0000-0000-0000-000000000001","name":"Chores"}}},"before":{"label":{"_0":{"color":"blue","cre\#
    atedAt":-978306200,"id":"01000000-0000-0000-0000-000000000001","name":"Errands"}}}},{"after":{"label\#
    ":{"_0":{"color":"blue","createdAt":-978306200,"id":"01000000-0000-0000-0000-000000000001","name":"E\#
    rrands"}}}}]}
    """#

private let savedConflict = #"""
    {"kind":{"fields":{"_0":["name"]}},"merged":{"label":{"_0":{"color":"blue","createdAt":-978306200,"i\#
    d":"01000000-0000-0000-0000-000000000001","name":"Chores"}}},"mine":{"label":{"_0":{"color":"blue","\#
    createdAt":-978306200,"id":"01000000-0000-0000-0000-000000000001","name":"Chores"}}},"record":{"id":\#
    "01000000-0000-0000-0000-000000000001","kind":"label"},"theirs":{"label":{"_0":{"color":"blue","crea\#
    tedAt":-978306200,"id":"01000000-0000-0000-0000-000000000001","name":"Jobs"}}}}
    """#

private let savedBases = #"""
    [{"id":"01000000-0000-0000-0000-000000000001","kind":"label"},{"label":{"_0":{"color":"blue","create\#
    dAt":-978306200,"id":"01000000-0000-0000-0000-000000000001","name":"Errands"}}}]
    """#

/// Reads `json` as `T` and writes it again with sorted keys.
private func roundTrip<T: Codable>(_ type: T.Type, _ json: String) throws -> (T, String) {
    let value = try JSONDecoder().decode(type, from: Data(json.utf8))
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    let written = try String(bytes: encoder.encode(value), encoding: .utf8).unwrap("expected UTF-8")
    return (value, written)
}

func savedFormatTests() -> [TestCase] {
    [
        TestCase("given a journal entry saved by an earlier Lifr, then it reads and writes the same") {
            let (entry, written) = try roundTrip(JournalEntry.self, savedEntry)
            try expectEqual(entry.steps.count, 2)
            try expectEqual(entry.steps[1].before, nil)
            try expectEqual(entry.reverts, [entry.conversation].compactMap(\.self))
            try expectEqual(written, savedEntry)
        },
        TestCase("given a sync conflict saved by an earlier Lifr, then it reads and writes the same") {
            let (conflict, written) = try roundTrip(SyncConflict.self, savedConflict)
            try expectEqual(conflict.kind, .fields([.name]))
            try expectEqual(written, savedConflict)
        },
        TestCase("given sync bases saved by an earlier Lifr, then they read and write the same") {
            let (bases, written) = try roundTrip([RecordID: RecordValue?].self, savedBases)
            try expectEqual(bases.keys.map(\.kind), [.label])
            try expectEqual(written, savedBases)
        },
        TestCase("given settings saved before the default reminder synced, then none is picked") {
            let (settings, written) = try roundTrip(Settings.self, #"{"procrastinatedAfterDays":3}"#)
            try expectEqual(settings, Settings(procrastinatedAfterDays: 3, defaultReminder: nil))
            try expectEqual(written, #"{"procrastinatedAfterDays":3}"#)
        },
    ]
}
