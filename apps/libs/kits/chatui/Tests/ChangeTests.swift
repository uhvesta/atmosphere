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
        TestCase("given no changes, then there are no groups") {
            try expect(ChangedField.byRecord([]).isEmpty)
        },
    ]
}
