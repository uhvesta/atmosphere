import Foundation
import LifrAPI
import LifrCore
import TestingKit

/// A table with every field the guide lists, each set to a value other
/// than its default, under `header` and the parents it needs.
private func table(_ path: [String], _ fields: [PlanFile.Field]) -> String {
    var lines = ["# Lifr plan v1"]
    for depth in path.indices {
        lines.append("[[\(path[...depth].joined(separator: "."))]]")
        if depth < path.count - 1 { lines.append("title = \"Parent\"") }
    }
    for field in fields {
        switch field.name {
        case "status": lines.append("status = \"done\"")
        case "priority": lines.append("priority = \"high\"")
        case _ where field.form == "a list": lines.append("\(field.name) = [\"x\"]")
        default: lines.append("\(field.name) = \"x\"")
        }
    }
    return lines.joined(separator: "\n") + "\n"
}

/// Why `text` doesn't read, or nil when it does.
private func readError(_ text: String) -> PlanFileError.Problem? {
    do {
        _ = try PlanFile.read(text)
        return nil
    } catch {
        return error.problem
    }
}

private let tables: [([String], [PlanFile.Field])] = [
    (["avenue"], PlanFile.avenueFields),
    (["label"], PlanFile.labelFields),
    (["project"], PlanFile.projectFields),
    (["task"], PlanFile.taskFields + PlanFile.placeFields),
    (["task", "subtask"], PlanFile.taskFields),
]

func planGuideTests() -> [TestCase] {
    [
        TestCase("given every field the guide lists, then the reader reads each one") {
            for (path, fields) in tables {
                let written = PlanFile.write(try PlanFile.read(table(path, fields)))
                for field in fields {
                    try expect(written.contains("\(field.name) = "), "\(path) dropped \(field.name)")
                }
            }
        },
        TestCase("given a field the guide calls required left out, then the reader asks for it") {
            for (path, fields) in tables {
                for field in fields where field.isRequired {
                    let problem = readError(table(path, fields.filter { $0 != field }))
                    guard case .missingKey(let key, _) = problem else {
                        try expect(false, "\(path) without \(field.name): \(String(describing: problem))")
                        continue
                    }
                    try expectEqual(key, field.name)
                }
            }
        },
        TestCase("given a field the guide doesn't list, then the reader refuses it") {
            for (path, fields) in tables {
                let problem = readError(table(path, fields + [PlanFile.Field(name: "colour")]))
                guard case .unknownKey(let key, _, _) = problem else {
                    try expect(false, "\(path) with colour: \(String(describing: problem))")
                    continue
                }
                try expectEqual(key, "colour")
            }
        },
        TestCase("given the guide's dates and repeats, then each one is read") {
            let today = try CalendarDate("2026-11-03")
            for form in PlanFile.dateForms {
                try expect(CalendarDate.resolving(form, today: today) != nil, form)
            }
            for form in PlanFile.repeatForms {
                try expect(Recurrence.resolving(form, due: today) != nil, form)
            }
        },
        TestCase("given the guide's reminders, then a plan using them passes its check") {
            let fixture = try Fixture()
            var task = Plan.TaskDraft(title: "Call the vet")
            task.place = "HEALTH"
            task.due = "tomorrow"
            task.reminders = PlanFile.reminderForms
            let problem = fixture.workspace.problem(with: Plan(tasks: [task]), timeZone: .gmt, now: fixture.now)
            try expectEqual(problem, nil)
        },
    ]
}
