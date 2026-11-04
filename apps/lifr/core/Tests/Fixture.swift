import Foundation
import LifrCore
import LifrTesting

/// A workspace with a HEALTH Avenue and a project in it, plus a context whose
/// IDs, codes and clock are fixed so every run is the same.
final class Fixture {
    var workspace = Workspace()
    var now = Date(timeIntervalSince1970: 1_793_664_000)  // 2026-11-03T00:00:00Z
    var actor = Actor.user(device: "iPhone", surface: .iPhone)
    private var ids = 0
    private var codes = 0
    let health: Avenue
    let marathon: Project

    init() throws {
        let context = Context(actor: actor, now: now)
        health = try workspace.createAvenue(key: "HEALTH", name: "Health", in: context)
        marathon = try workspace.createProject(name: "Half marathon", inAvenue: health.id, in: context)
    }

    var context: Context {
        Context(
            actor: actor, now: now, newID: { [unowned self] in self.nextID() },
            newCode: { [unowned self] in self.nextCode() })
    }

    func nextID() -> UUID {
        ids += 1
        return UUID(sequence: ids)
    }

    func nextCode() -> TaskCode {
        codes += 1
        return TaskCode(sequence: codes)
    }

    func advance(days: Double) {
        now += days * 24 * 60 * 60
    }

    @discardableResult
    func task(
        _ title: String, status: TaskStatus = .todo, parent: LifrTask? = nil,
        project: Project? = nil
    ) throws -> LifrTask {
        var new = Workspace.NewTask(title: title, avenueID: health.id, projectID: project?.id, parentID: parent?.id)
        new.status = status
        let change = try workspace.createTask(new, in: context)
        let id = try change.previous.first { $0.value == nil }.unwrap("creating a task made no new one").key
        return try workspace.tasks[id].unwrap("the new task isn't in the workspace")
    }

    /// A second device with the same records, as after a sync. It continues
    /// this fixture's ID sequence so new records don't reuse IDs, but hands
    /// out the same next code, as two offline devices can.
    init(copying other: Fixture, device: String) {
        workspace = other.workspace
        now = other.now
        actor = .user(device: device, surface: .mac)
        ids = other.ids + 1000
        codes = other.codes
        health = other.health
        marathon = other.marathon
    }

    func get(_ task: LifrTask) throws -> LifrTask {
        try workspace.tasks[task.id].unwrap("\(task.title) isn't in the workspace")
    }
}

/// `LifrTesting.expectError` for Core's errors, so cases can be written as
/// `.openSubtasks(...)`.
func expectError<T>(
    _ expected: LifrError, file: StaticString = #filePath, line: UInt = #line,
    _ body: () throws -> T
) throws {
    try LifrTesting.expectError(expected, file: file, line: line, body)
}
