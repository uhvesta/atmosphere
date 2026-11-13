import Foundation
import LifrCore
import TestingKit

func focusTests() -> [TestCase] {
    [
        TestCase("given a Focus on one Avenue, then only its Projects, tasks and reminders show") {
            let fixture = try Fixture()
            let home = try fixture.workspace.createAvenue(key: "HOME", name: "Home", in: fixture.context)
            let run = try fixture.task("Run", project: fixture.marathon)
            _ = try fixture.workspace.createTask(
                Workspace.NewTask(title: "Fix the sink", avenueID: home.id), in: fixture.context)
            let focused = fixture.workspace.focused(on: FocusAvenues([fixture.health.id]))
            try expectEqual(Set(focused.avenues.keys), [fixture.health.id])
            try expectEqual(Set(focused.projects.keys), [fixture.marathon.id])
            try expectEqual(Set(focused.tasks.keys), [run.id])
        },
        TestCase("given no Focus, then every record shows") {
            let fixture = try Fixture()
            try fixture.task("Run")
            try expectEqual(fixture.workspace.focused(on: FocusAvenues()), fixture.workspace)
        },
        TestCase("given the Avenues a Focus saved, when they're read back, then they're the same") {
            let ids: Set<UUID> = [UUID(sequence: 1), UUID(sequence: 2)]
            try expectEqual(FocusAvenues(rawValue: FocusAvenues(ids).rawValue), FocusAvenues(ids))
            try expectEqual(FocusAvenues(rawValue: "not an id").ids, [])
        },
    ]
}
