import Foundation
import LifrCore
import TestingKit

extension Fixture {
    /// Saves `filter` the way the stores do, with a journal entry.
    @discardableResult
    func saveFilter(_ filter: TaskFilter, name: String = "Errands") throws -> (SavedFilter, Change) {
        var saved: SavedFilter?
        let change = try perform { workspace, context throws(LifrError) in
            saved = try workspace.saveFilter(filter, name: name, symbol: "cart", in: context)
            return Change()
        }
        return (try saved.unwrap("saving made no filter"), change)
    }
}

private func savedFilter(_ value: RecordValue?) throws -> SavedFilter {
    guard case .savedFilter(let saved)? = value else {
        throw Unexpected("expected a saved filter, got \(String(describing: value))")
    }
    return saved
}

func savedFilterTests() -> [TestCase] {
    [
        TestCase("given a filter, when it's saved, then it's listed last with a trimmed name") {
            let fixture = try Fixture()
            let (first, _) = try fixture.saveFilter(TaskFilter(text: "milk"), name: "Shopping")
            let (second, _) = try fixture.saveFilter(TaskFilter(priorities: [.high]), name: "  Urgent ")
            try expectEqual(fixture.workspace.sortedSavedFilters.map(\.id), [first.id, second.id])
            try expectEqual(second.name, "Urgent")
        },
        TestCase("given no words and no filters, when it's saved, then it's refused") {
            let fixture = try Fixture()
            try expectError(LifrError.emptyFilter) { try fixture.saveFilter(TaskFilter()) }
            try expectEqual(fixture.workspace.savedFilters.count, 0)
        },
        TestCase("given a saved filter, when it's edited, then its name, symbol and filter change") {
            let fixture = try Fixture()
            let (saved, _) = try fixture.saveFilter(TaskFilter(text: "milk"))
            try fixture.workspace.edit(
                savedFilter: saved.id, name: "Dairy", symbol: "star", filter: TaskFilter(text: "cheese"))
            let edited = try fixture.workspace.savedFilters[saved.id].unwrap()
            try expectEqual(edited.name, "Dairy")
            try expectEqual(edited.symbol, "star")
            try expectEqual(edited.filter.text, "cheese")
        },
        TestCase("given a saved filter, when it's moved before another, then the order changes") {
            let fixture = try Fixture()
            let (first, _) = try fixture.saveFilter(TaskFilter(text: "a"))
            let (second, _) = try fixture.saveFilter(TaskFilter(text: "b"))
            try fixture.workspace.reorder(savedFilter: second.id, after: nil, before: first.id)
            try expectEqual(fixture.workspace.sortedSavedFilters.map(\.id), [second.id, first.id])
        },
        TestCase("given a missing saved filter, when it's changed, then it's not found") {
            let fixture = try Fixture()
            let id = UUID()
            try expectError(LifrError.notFound(.savedFilter, id.uuidString)) {
                try fixture.workspace.delete(savedFilter: id)
            }
        },
        TestCase("given a deleted saved filter, when its deletion is reverted, then it's back") {
            let fixture = try Fixture()
            let (saved, _) = try fixture.saveFilter(TaskFilter(text: "milk"))
            let deletion = try fixture.perform { workspace, _ throws(LifrError) in
                try workspace.delete(savedFilter: saved.id)
                return Change()
            }
            try expectEqual(fixture.workspace.savedFilters[saved.id], nil)
            _ = try fixture.revert([deletion.entry])
            try expectEqual(fixture.workspace.savedFilters[saved.id], saved)
        },
        TestCase("given a saved filter, when saving it is reverted, then it's removed") {
            let fixture = try Fixture()
            let (saved, change) = try fixture.saveFilter(TaskFilter(text: "milk"))
            _ = try fixture.revert([change.entry])
            try expectEqual(fixture.workspace.savedFilters[saved.id], nil)
        },
        TestCase("given a filter on a label, when the label is renamed, then the filter follows it") {
            let fixture = try Fixture()
            let label = fixture.workspace.createLabel(name: "Errand", in: fixture.context)
            let (saved, _) = try fixture.saveFilter(TaskFilter(labels: ["Errand", "Phone"]))
            try fixture.workspace.rename(label: label.id, to: "Errands")
            try expectEqual(fixture.workspace.savedFilters[saved.id]?.filter.labels, ["Errands", "Phone"])
        },
        TestCase("given a filter on a label, when it's merged into another, then the filter has the other") {
            let fixture = try Fixture()
            let errand = fixture.workspace.createLabel(name: "Errand", in: fixture.context)
            let out = fixture.workspace.createLabel(name: "Out", in: fixture.context)
            let (saved, _) = try fixture.saveFilter(TaskFilter(text: "x", labels: ["Errand"]))
            _ = try fixture.workspace.merge(label: errand.id, into: out.id, in: fixture.context)
            try expectEqual(fixture.workspace.savedFilters[saved.id]?.filter.labels, ["Out"])
        },
        TestCase("given a filter on a label, when the label is deleted, then the filter drops it") {
            let fixture = try Fixture()
            let errand = fixture.workspace.createLabel(name: "Errand", in: fixture.context)
            let (saved, _) = try fixture.saveFilter(TaskFilter(text: "x", labels: ["Errand"]))
            _ = try fixture.workspace.merge(label: errand.id, into: nil, in: fixture.context)
            try expectEqual(fixture.workspace.savedFilters[saved.id]?.filter.labels, [])
        },
        TestCase("given a saved filter, when it's encoded and decoded, then it's the same") {
            let fixture = try Fixture()
            let filter = TaskFilter(
                text: "milk", statuses: [.todo], priorities: [.high], labels: ["Errand"],
                avenues: [fixture.health.id], due: .nextSevenDays)
            let (saved, _) = try fixture.saveFilter(filter)
            let data = try JSONEncoder().encode(RecordValue.savedFilter(saved))
            try expectEqual(try JSONDecoder().decode(RecordValue.self, from: data), .savedFilter(saved))
        },
        TestCase("given each device changed a different field of a saved filter, then both are kept") {
            let fixture = try Fixture()
            let (original, _) = try fixture.saveFilter(TaskFilter(text: "milk"))
            var mine = original
            mine.name = "Dairy"
            var theirs = original
            theirs.filter.text = "cheese"
            let result = RecordValue.merge(
                base: .savedFilter(original), mine: .savedFilter(mine), theirs: .savedFilter(theirs))
            guard case .merged(let value) = result else { throw Unexpected("expected a merge, got \(result)") }
            let merged = try savedFilter(value)
            try expectEqual(merged.name, "Dairy")
            try expectEqual(merged.filter.text, "cheese")
        },
        TestCase("given both devices renamed a saved filter differently, then the name conflicts") {
            let fixture = try Fixture()
            let (original, _) = try fixture.saveFilter(TaskFilter(text: "milk"))
            var mine = original
            mine.name = "Dairy"
            var theirs = original
            theirs.name = "Milk run"
            let result = RecordValue.merge(
                base: .savedFilter(original), mine: .savedFilter(mine), theirs: .savedFilter(theirs))
            guard case .conflict(let conflict) = result else { throw Unexpected("expected a conflict, got \(result)") }
            try expectEqual(conflict.kind, .fields([.name]))
        },
        TestCase("given a filter, then a task matches only when every set filter does") {
            let fixture = try Fixture()
            let task = try fixture.task("Buy milk")
            let now = fixture.now
            let matches = { (filter: TaskFilter) in
                filter.matches(task, in: fixture.workspace, now: now, timeZone: .gmt)
            }
            try expect(matches(TaskFilter(text: "MILK")))
            try expect(!matches(TaskFilter(text: "milk bread")))
            try expect(matches(TaskFilter(text: "milk", avenues: [fixture.health.id])))
            try expect(!matches(TaskFilter(text: "milk", priorities: [.high])))
            try expect(matches(TaskFilter(due: .none)))
            try expect(!matches(TaskFilter(due: .today)))
        },
    ]
}
