import Foundation
import LifrCore

// The sample's values are fixed, so a failing `try` is a bug in the sample.
// swiftlint:disable force_try

extension Workspace {
    /// A few Avenues with tasks due around `now`, for previews, snapshot
    /// tests and the app until it stores records. The same `now` gives the
    /// same records.
    package static func sample(now: Date, timeZone: TimeZone) -> Workspace {
        var records = RecordsBuilder(now: now, timeZone: timeZone)
        let health = records.avenue("HEALTH", "Health", color: "green", symbol: "heart")
        let work = records.avenue("WORK", "Work", color: "blue", symbol: "briefcase")
        let home = records.avenue("HOME", "Home", color: "orange", symbol: "house")

        records.task("Long run, 10 km", in: health, priority: .high, due: 0, at: "07:00")
        records.task("Book physio appointment", in: health, priority: .medium, due: -2)
        let review = records.task("Review Q4 roadmap", in: work, status: .inProgress, priority: .urgent, due: -1)
        records.task("Read the draft", in: work, parent: review, status: .done)
        records.task("Leave comments", in: work, parent: review)
        records.task("Send summary to the team", in: work, parent: review)
        records.task("Water the plants", in: home, due: 0)
        records.task("Fix the bike's back brake", in: home, status: .procrastinated, priority: .low, due: -5)
        records.task("Renew passport", in: home, priority: .medium, due: 6)
        records.task("Plan long runs", in: health, status: .inProgress, idle: 12)
        records.task("Call the plumber", in: home, due: 2)
        let marathon = records.project("Half marathon", in: health)
        records.task("Buy running shoes", in: health, project: marathon, priority: .high, due: 1)
        records.reminder(.at(now + 2 * 60 * 60), on: review)
        return records.workspace
    }
}

/// Builds records with fixed IDs and codes, for samples and tests. Each record
/// gets the next ID and code, so the same calls give the same records.
package struct RecordsBuilder {
    package private(set) var workspace = Workspace()
    package let now: Date
    package let timeZone: TimeZone
    private var ids = 0
    private var codes = 0

    package init(now: Date, timeZone: TimeZone) {
        self.now = now
        self.timeZone = timeZone
    }

    package mutating func avenue(
        _ key: String, _ name: String, color: String = "blue", symbol: String = "circle"
    ) -> Avenue {
        try! workspace.createAvenue(key: key, name: name, color: color, symbol: symbol, in: context())
    }

    package mutating func project(_ name: String, in avenue: Avenue) -> Project {
        try! workspace.createProject(name: name, inAvenue: avenue.id, in: context())
    }

    /// `due` counts days from `now`'s day: 0 is today, -1 yesterday.
    /// `idle` makes the task that many days before `now`, with no activity
    /// since.
    @discardableResult
    package mutating func task(
        _ title: String, in avenue: Avenue, project: Project? = nil, parent: LifrTask? = nil,
        status: TaskStatus = .todo,
        priority: Priority = .none, due days: Int? = nil, at time: String? = nil, idle: Int = 0,
        labels: [Label] = []
    ) -> LifrTask {
        var new = Workspace.NewTask(title: title, avenueID: avenue.id, parentID: parent?.id)
        new.projectID = project?.id
        new.labelIDs = Set(labels.map(\.id))
        new.status = status
        new.priority = priority
        new.dueDate = days.map { CalendarDate(now + Double($0) * 24 * 60 * 60, in: timeZone) }
        new.dueTime = time.map { try! TimeOfDay($0) }
        let change = try! workspace.createTask(new, in: context(at: now - Double(idle) * 24 * 60 * 60))
        return workspace.tasks[change.previous.first { $0.value == nil }!.key]!
    }

    package mutating func label(_ name: String) -> Label {
        workspace.createLabel(name: name, in: context())
    }

    package mutating func comment(_ body: String, on task: LifrTask) {
        _ = try! workspace.addComment(body, to: task.id, in: context())
    }

    @discardableResult
    package mutating func reminder(_ trigger: Reminder.Trigger, on task: LifrTask) -> Reminder {
        try! workspace.addReminder(trigger, to: task.id, in: context())
    }

    package mutating func setStatus(_ status: TaskStatus, of task: LifrTask) {
        _ = try! workspace.setStatus(status, of: task.id, in: context())
    }

    private mutating func context(at time: Date? = nil) -> Context {
        ids += 100
        codes += 1
        let id = ids
        let code = codes
        var next = 0
        return Context(
            actor: .user(device: "Sample", surface: .iPhone), now: time ?? now,
            newID: {
                next += 1
                return UUID(uuidString: String(format: "00000000-0000-0000-0000-%012d", id + next))!
            },
            newCode: { try! TaskCode(String(format: "%04d", code)) })
    }
}
// swiftlint:enable force_try
