import Foundation
import LifrCore
import TestingKit

func backupTests() -> [TestCase] {
    [
        TestCase("given an export, when it's imported into an empty workspace, then the same records are there") {
            let fixture = try Fixture()
            let run = try fixture.task("Run", project: fixture.marathon)
            try fixture.task("Stretch", parent: run)
            _ = try fixture.workspace.addComment("Felt good", to: run.id, in: fixture.context)
            let data = try fixture.workspace.exported(at: fixture.now).encoded()

            var empty = Workspace()
            let plan = try empty.importPlan(LifrExport.decoded(from: data))
            _ = empty.apply(plan, in: fixture.context)
            try expectEqual(empty.tasks, fixture.workspace.tasks)
            try expectEqual(empty.projects, fixture.workspace.projects)
            try expectEqual(empty.comments, fixture.workspace.comments)
            try expectEqual(Set(empty.history.map(\.id)), Set(fixture.workspace.history.map(\.id)))
        },
        TestCase("given an export, when it's imported where it came from, then nothing is added") {
            let fixture = try Fixture()
            try fixture.task("Run")
            let export = fixture.workspace.exported(at: fixture.now)
            let plan = try fixture.workspace.importPlan(LifrExport.decoded(from: export.encoded()))
            try expect(plan.isEmpty)
            try expectEqual(plan.alreadyHere, export.records.count)
        },
        TestCase("given a task deleted since an export, when the export is imported, then only that task is added") {
            let fixture = try Fixture()
            try fixture.task("Run")
            let before = Fixture(copying: fixture, device: "Mac")
            let swim = try fixture.task("Swim")
            let plan = try before.workspace.importPlan(fixture.workspace.exported(at: fixture.now))
            try expectEqual(plan.records, [.task(swim)])
            try expect(!plan.history.isEmpty && plan.history.allSatisfy { $0.taskID == swim.id })
        },
        TestCase("given a task whose Avenue isn't in the file or here, when imported, then that record is named") {
            let fixture = try Fixture()
            let run = try fixture.task("Run")
            var export = fixture.workspace.exported(at: fixture.now)
            export.records.removeAll { if case .avenue = $0 { true } else { false } }
            export.records.removeAll { if case .project = $0 { true } else { false } }
            var task = run
            task.projectID = nil
            export.records = export.records.map { if case .task = $0 { .task(task) } else { $0 } }
            let number = try export.records.firstIndex(of: .task(task)).unwrap() + 1
            try expectError(ImportError.badRecord(number: number, name: "Run", .missingAvenue)) {
                try Workspace().importPlan(export)
            }
        },
        TestCase("given a file with a broken record, when it's read, then that record's place is named") {
            let fixture = try Fixture()
            try fixture.task("Run")
            let data = try fixture.workspace.exported(at: fixture.now).encoded()
            var json = try (JSONSerialization.jsonObject(with: data) as? [String: Any]).unwrap()
            var records = try (json["records"] as? [Any]).unwrap()
            records[0] = ["avenue": ["_0": ["name": "No ID"]]]
            json["records"] = records
            let broken = try JSONSerialization.data(withJSONObject: json)
            let error = importError { () throws(ImportError) in try LifrExport.decoded(from: broken) }
            guard case .badRecord(number: 1, name: "No ID", .unreadable)? = error else {
                return try expect(false, "got \(String(describing: error))")
            }
        },
        TestCase("given text that isn't JSON, when it's read, then it's unreadable") {
            let text = Data("{\n\"format\": 1,\n oops".utf8)
            let error = importError { () throws(ImportError) in try LifrExport.decoded(from: text) }
            guard case .unreadable? = error else { return try expect(false, "got \(String(describing: error))") }
        },
        TestCase("given a file from a newer Lifr, when it's read, then newerFormat") {
            let export = LifrExport(format: LifrExport.currentFormat + 1, exportedAt: .now, records: [], history: [])
            try expectError(ImportError.newerFormat(LifrExport.currentFormat + 1)) {
                try LifrExport.decoded(from: export.encoded())
            }
        },
        TestCase("given quoted CSV fields, when parsed, then commas, quotes and line breaks stay in them") {
            let table = try CSVTable(parsing: "Title,Notes\n\"Buy milk, eggs\",\"Say \"\"hi\"\"\nthen go\"\n\nRun,\n")
            try expectEqual(table.columns, ["Title", "Notes"])
            try expectEqual(table.rows.map(\.fields), [["Buy milk, eggs", "Say \"hi\"\nthen go"], ["Run", ""]])
            try expectEqual(table.rows.map(\.line), [2, 5])
        },
        TestCase("given an unclosed quote, when CSV is parsed, then its line is named") {
            try expectError(ImportError.badLine(2, .unclosedQuote)) {
                try CSVTable(parsing: "Title\n\"Run\nSwim\n")
            }
        },
        TestCase("given CSV rows, when they're mapped, then each makes a task with its Avenue, due date and status") {
            let fixture = try Fixture()
            let table = try CSVTable(parsing: "Name,Due,List,Status\nRun,2026-11-20,health,In Progress\nCall Mum,,,\n")
            let mapping = try CSVMapping(guessingFrom: table.columns).unwrap()
            let tasks = try fixture.workspace.newTasks(from: table, mapping: mapping)
            try expectEqual(tasks.map(\.title), ["Run", "Call Mum"])
            try expectEqual(tasks.map(\.avenueID), [fixture.health.id, Avenue.inboxID])
            try expectEqual(tasks.map(\.dueDate), [try CalendarDate(year: 2026, month: 11, day: 20), nil])
            try expectEqual(tasks.map(\.status), [.inProgress, .todo])
        },
        TestCase("given a CSV row with a bad date, when it's mapped, then its line is named") {
            let fixture = try Fixture()
            let table = try CSVTable(parsing: "Title,Due\nRun,2026-11-20\nSwim,next week\n")
            let mapping = CSVMapping(title: 0, due: 1)
            try expectError(ImportError.badLine(3, .badDate("next week"))) {
                try fixture.workspace.newTasks(from: table, mapping: mapping)
            }
        },
        TestCase("given an Avenue's tasks, when it's Markdown, then open and closed tasks nest under their Project") {
            let fixture = try Fixture()
            let run = try fixture.task("Run", project: fixture.marathon)
            try fixture.task("Stretch", status: .done, parent: run)
            try fixture.task("Book physio")
            let markdown = try fixture.workspace.markdown(of: fixture.health.id).unwrap()
            let lines = markdown.split(separator: "\n").map(String.init)
            try expectEqual(lines.first, "# Health")
            let physio = try lines.firstIndex { $0.hasPrefix("- [ ]") && $0.hasSuffix("Book physio") }.unwrap()
            let project = try lines.firstIndex(of: "## Half marathon").unwrap()
            let runLine = try lines.firstIndex { $0.hasPrefix("- [ ]") && $0.hasSuffix("Run") }.unwrap()
            let stretch = try lines.firstIndex { $0.hasPrefix("  - [x]") && $0.contains("Stretch") }.unwrap()
            try expect(physio < project && project < runLine && runLine < stretch)
        },
    ]
}

/// The error `body` throws, or `nil` when it doesn't.
private func importError(_ body: () throws(ImportError) -> some Any) -> ImportError? {
    do {
        _ = try body()
        return nil
    } catch {
        return error
    }
}
