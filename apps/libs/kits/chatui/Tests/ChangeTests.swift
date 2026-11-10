import ChatUI
import TestingKit

func changeTests() -> [TestCase] {
    [
        TestCase("given changes to two records in turn, then each run of one record is a group, in order") {
            let changes = [
                ChangedField(record: "Run", field: "Status", before: "To Do", after: "Done"),
                ChangedField(record: "Run", field: "Due", before: nil, after: "Friday"),
                ChangedField(record: "Swim", field: "Status", before: nil, after: "To Do"),
                ChangedField(record: "Run", field: "Title", before: "Run", after: "Run 5 km"),
            ]
            let groups = ChangedField.byRecord(changes)
            try expectEqual(groups.map(\.record), ["Run", "Swim", "Run"])
            try expectEqual(groups.map(\.fields.count), [2, 1, 1])
        },
        TestCase("given a task made with some fields empty, then only what it was made with, without arrows") {
            let lines = ChangedField.lines([
                ChangedField(record: "Buy oat milk", field: "Title", before: nil, after: "Buy oat milk"),
                ChangedField(record: "Buy oat milk", field: "Details", before: nil, after: ""),
                ChangedField(record: "Buy oat milk", field: "Status", before: nil, after: "To Do"),
                ChangedField(record: "Buy oat milk", field: "Labels", before: nil, after: " "),
            ])
            try expectEqual(lines, ["Title: Buy oat milk", "Status: To Do"])
        },
        TestCase("given an edit, then each field before and after, with a dash for what was empty") {
            let lines = ChangedField.lines([
                ChangedField(record: "Run", field: "Status", before: "To Do", after: "Done"),
                ChangedField(record: "Run", field: "Closed", before: nil, after: "Nov 3"),
            ])
            try expectEqual(lines, ["Status: To Do → Done", "Closed: — → Nov 3"])
        },
        TestCase("given one field set from empty, then it still shows what it was") {
            let lines = ChangedField.lines([ChangedField(record: "Run", field: "Due", before: nil, after: "Friday")])
            try expectEqual(lines, ["Due: — → Friday"])
        },
        TestCase("given changes to two records, then each names its record") {
            let lines = ChangedField.lines([
                ChangedField(record: "Run", field: "Status", before: "To Do", after: "Done"),
                ChangedField(record: "Swim", field: "Status", before: "Done", after: "To Do"),
            ])
            try expectEqual(lines, ["Run · Status: To Do → Done", "Swim · Status: Done → To Do"])
        },
        TestCase("given no changes, then there are no groups") {
            try expect(ChangedField.byRecord([]).isEmpty)
        },
    ]
}
