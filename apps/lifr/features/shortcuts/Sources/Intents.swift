import AppIntents
import Foundation
import LifrCore

/// A task, for Shortcuts and Siri.
package struct TaskEntity: AppEntity {
    package static let typeDisplayRepresentation: TypeDisplayRepresentation = "Task"
    package static let defaultQuery = TaskQuery()

    package var id: UUID
    package var title: String
    package var code: String

    package var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(title)", subtitle: "\(code)")
    }

    init(_ item: Shortcuts.Item) {
        id = item.id
        title = item.name
        code = item.detail
    }

    /// Open tasks, Today's first. None while Lifr's lock is on.
    package struct TaskQuery: EntityStringQuery {
        @AppDependency private var shortcuts: Shortcuts

        package init() {}

        package func entities(for identifiers: [UUID]) async throws -> [TaskEntity] {
            try await shortcuts.tasks(identifiers).map(TaskEntity.init)
        }

        package func entities(matching string: String) async throws -> [TaskEntity] {
            try await shortcuts.openTasks(matching: string).map(TaskEntity.init)
        }

        package func suggestedEntities() async throws -> [TaskEntity] {
            try await shortcuts.openTasks(matching: nil).map(TaskEntity.init)
        }
    }
}

/// An Avenue, for Shortcuts and Siri.
package struct AvenueEntity: AppEntity {
    package static let typeDisplayRepresentation: TypeDisplayRepresentation = "Avenue"
    package static let defaultQuery = AvenueQuery()

    package var id: UUID
    package var name: String

    package var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(name)")
    }

    package struct AvenueQuery: EntityQuery {
        @AppDependency private var shortcuts: Shortcuts

        package init() {}

        package func entities(for identifiers: [UUID]) async throws -> [AvenueEntity] {
            try await shortcuts.avenues().filter { identifiers.contains($0.id) }.map {
                AvenueEntity(id: $0.id, name: $0.name)
            }
        }

        package func suggestedEntities() async throws -> [AvenueEntity] {
            try await shortcuts.avenues().map { AvenueEntity(id: $0.id, name: $0.name) }
        }
    }
}

/// When a task added from Shortcuts is due.
package enum DueOption: String, AppEnum {
    case today, tomorrow, none

    package static let typeDisplayRepresentation: TypeDisplayRepresentation = "Due"
    package static let caseDisplayRepresentations: [DueOption: DisplayRepresentation] = [
        .today: "Today", .tomorrow: "Tomorrow", .none: "No date",
    ]

    var due: Shortcuts.Due {
        switch self {
        case .today: .today
        case .tomorrow: .tomorrow
        case .none: .none
        }
    }
}

/// Adds a task without opening Lifr.
package struct AddTaskIntent: AppIntent {
    package static let title: LocalizedStringResource = "Add a Task"
    package static let description = IntentDescription("Adds a task to Lifr, due today unless you pick otherwise.")

    @Parameter(title: "Title", requestValueDialog: "What should Lifr add?")
    package var taskTitle: String
    @Parameter(title: "Avenue")
    package var avenue: AvenueEntity?
    @Parameter(title: "Due", default: .today)
    package var due: DueOption

    @AppDependency private var shortcuts: Shortcuts

    package init() {}

    package static var parameterSummary: some ParameterSummary {
        Summary("Add \(\.$taskTitle) to Lifr") {
            \.$avenue
            \.$due
        }
    }

    package func perform() async throws -> some IntentResult & ProvidesDialog {
        let task = try await shortcuts.add(taskTitle, avenue: avenue?.id, due: due.due)
        return .result(dialog: "Added \(task.name).")
    }
}

/// Says what's on Today: the tasks by name, or only how many while Lifr's
/// lock is on.
package struct TodayIntent: AppIntent {
    package static let title: LocalizedStringResource = "What's on Today"
    package static let description = IntentDescription(
        "Says how many tasks are due today or overdue, and names them unless Lifr's lock is on.")

    @AppDependency private var shortcuts: Shortcuts

    package init() {}

    package func perform() async throws -> some IntentResult & ProvidesDialog & ReturnsValue<Int> {
        let today = try await shortcuts.today()
        return .result(value: today.count, dialog: IntentDialog(stringLiteral: TodaySummary(today).text))
    }
}

/// Marks a task done.
package struct FinishTaskIntent: AppIntent {
    package static let title: LocalizedStringResource = "Finish a Task"
    package static let description = IntentDescription("Marks a task in Lifr as done.")

    @Parameter(title: "Task", requestValueDialog: "Which task did you finish?")
    package var task: TaskEntity

    @AppDependency private var shortcuts: Shortcuts

    package init() {}

    package static var parameterSummary: some ParameterSummary {
        Summary("Finish \(\.$task)")
    }

    package func perform() async throws -> some IntentResult & ProvidesDialog {
        let task = try await shortcuts.finish(task.id)
        return .result(dialog: "Finished \(task.name).")
    }
}

/// A status, for Shortcuts and Siri.
package enum StatusOption: String, AppEnum {
    case todo, inProgress, procrastinated, done, wontDo

    package static let typeDisplayRepresentation: TypeDisplayRepresentation = "Status"
    package static let caseDisplayRepresentations: [StatusOption: DisplayRepresentation] = [
        .todo: "To Do", .inProgress: "In Progress", .procrastinated: "Procrastinated", .done: "Done",
        .wontDo: "Won't Do",
    ]

    var status: TaskStatus {
        switch self {
        case .todo: .todo
        case .inProgress: .inProgress
        case .procrastinated: .procrastinated
        case .done: .done
        case .wontDo: .wontDo
        }
    }
}

/// Changes a task's status, which Shortcuts asks for when it isn't set.
package struct SetStatusIntent: AppIntent {
    package static let title: LocalizedStringResource = "Change a Task's Status"
    package static let description = IntentDescription(
        "Sets a task in Lifr to To Do, In Progress, Done or another status.")

    @Parameter(title: "Task", requestValueDialog: "Which task?")
    package var task: TaskEntity
    @Parameter(title: "Status", requestValueDialog: "Which status?")
    package var status: StatusOption

    @AppDependency private var shortcuts: Shortcuts

    package init() {}

    package static var parameterSummary: some ParameterSummary {
        Summary("Set \(\.$task) to \(\.$status)")
    }

    package func perform() async throws -> some IntentResult & ProvidesDialog {
        let task = try await shortcuts.setStatus(status.status, of: task.id)
        return .result(dialog: "Changed the status of \(task.name).")
    }
}

/// Moves a task's due date.
package struct RescheduleIntent: AppIntent {
    package static let title: LocalizedStringResource = "Reschedule a Task"
    package static let description = IntentDescription("Moves a task in Lifr to another day, or leaves it with none.")

    @Parameter(title: "Task", requestValueDialog: "Which task?")
    package var task: TaskEntity
    @Parameter(title: "Due", kind: .date)
    package var date: Date?

    @AppDependency private var shortcuts: Shortcuts

    package init() {}

    package static var parameterSummary: some ParameterSummary {
        Summary("Move \(\.$task) to \(\.$date)")
    }

    package func perform() async throws -> some IntentResult & ProvidesDialog {
        let task = try await shortcuts.reschedule(task.id, to: date)
        return .result(dialog: date == nil ? "\(task.name) has no due date now." : "Moved \(task.name).")
    }
}

/// Adds a comment to a task.
package struct AddCommentIntent: AppIntent {
    package static let title: LocalizedStringResource = "Comment on a Task"
    package static let description = IntentDescription("Adds a comment to a task in Lifr.")

    @Parameter(title: "Task", requestValueDialog: "Which task?")
    package var task: TaskEntity
    @Parameter(title: "Comment", requestValueDialog: "What's the comment?")
    package var text: String

    @AppDependency private var shortcuts: Shortcuts

    package init() {}

    package static var parameterSummary: some ParameterSummary {
        Summary("Comment \(\.$text) on \(\.$task)")
    }

    package func perform() async throws -> some IntentResult & ProvidesDialog {
        let task = try await shortcuts.comment(text, on: task.id)
        return .result(dialog: "Added your comment to \(task.name).")
    }
}

/// Opens a task in Lifr, after the lock if it's on.
package struct OpenTaskIntent: AppIntent {
    package static let title: LocalizedStringResource = "Open a Task"
    package static let description = IntentDescription("Opens a task in Lifr.")
    package static let openAppWhenRun = true

    @Parameter(title: "Task", requestValueDialog: "Which task?")
    package var task: TaskEntity

    package init() {}

    package static var parameterSummary: some ParameterSummary {
        Summary("Open \(\.$task)")
    }

    @MainActor
    package func perform() async throws -> some IntentResult & OpensIntent {
        var open = OpenURLIntent()
        if let url = DeepLink.task(task.id, picksStatus: false).url { open.url = url }
        return .result(opensIntent: open)
    }
}

/// Makes a prompt to plan something with another chat assistant, for a
/// shortcut to copy or pass on. Paste the assistant's reply into a Lifr chat
/// with /paste to review the plan.
package struct PlanPromptIntent: AppIntent {
    package static let title: LocalizedStringResource = "Make a Plan Prompt"
    package static let description = IntentDescription(
        """
        Makes a prompt asking another chat assistant for a plan Lifr can read. It names your Avenues, \
        Projects and labels, but no tasks. Paste the reply into a Lifr chat with /paste.
        """)

    @Parameter(title: "Goal", requestValueDialog: "What do you want to plan?")
    package var goal: String

    @AppDependency private var shortcuts: Shortcuts

    package init() {}

    package static var parameterSummary: some ParameterSummary {
        Summary("Make a prompt to plan \(\.$goal)")
    }

    package func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await shortcuts.planPrompt(goal: goal))
    }
}

/// What Siri offers without setting anything up.
package struct LifrAppShortcuts: AppShortcutsProvider {
    package static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: AddTaskIntent(),
            phrases: ["Add a task in \(.applicationName)", "Add to \(.applicationName)"],
            shortTitle: "Add a Task", systemImageName: "plus.circle")
        AppShortcut(
            intent: TodayIntent(),
            phrases: ["What's on today in \(.applicationName)", "What's due in \(.applicationName)"],
            shortTitle: "Today", systemImageName: "star")
        AppShortcut(
            intent: FinishTaskIntent(),
            phrases: ["Finish a task in \(.applicationName)", "Finish \(\.$task) in \(.applicationName)"],
            shortTitle: "Finish a Task", systemImageName: "checkmark.circle")
        AppShortcut(
            intent: SetStatusIntent(),
            phrases: ["Change a task's status in \(.applicationName)", "Set \(\.$task) in \(.applicationName)"],
            shortTitle: "Change Status", systemImageName: "circle.dotted")
        AppShortcut(
            intent: OpenTaskIntent(),
            phrases: ["Open a task in \(.applicationName)", "Open \(\.$task) in \(.applicationName)"],
            shortTitle: "Open a Task", systemImageName: "arrow.up.forward.app")
    }
}
