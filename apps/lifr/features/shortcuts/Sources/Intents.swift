import AppIntents
import Foundation
import LifrCore
import LifrPasteboardClient
import LifrUI

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
        return .result(dialog: "Marked \(task.name) \(status.status.title).")
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

/// A Project, for Shortcuts and Siri.
package struct ProjectEntity: AppEntity {
    package static let typeDisplayRepresentation: TypeDisplayRepresentation = "Project"
    package static let defaultQuery = ProjectQuery()

    package var id: UUID
    package var name: String
    package var avenue: String

    package var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(name)", subtitle: "\(avenue)")
    }

    init(_ item: Shortcuts.Item) {
        id = item.id
        name = item.name
        avenue = item.detail
    }

    /// The active Projects.
    package struct ProjectQuery: EntityQuery {
        @AppDependency private var shortcuts: Shortcuts

        package init() {}

        package func entities(for identifiers: [UUID]) async throws -> [ProjectEntity] {
            try await shortcuts.projects().filter { identifiers.contains($0.id) }.map(ProjectEntity.init)
        }

        package func suggestedEntities() async throws -> [ProjectEntity] {
            try await shortcuts.projects().map(ProjectEntity.init)
        }
    }
}

/// A label, for Shortcuts and Siri.
package struct LabelEntity: AppEntity {
    package static let typeDisplayRepresentation: TypeDisplayRepresentation = "Label"
    package static let defaultQuery = LabelQuery()

    package var id: UUID
    package var name: String

    package var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(name)")
    }

    package struct LabelQuery: EntityQuery {
        @AppDependency private var shortcuts: Shortcuts

        package init() {}

        package func entities(for identifiers: [UUID]) async throws -> [LabelEntity] {
            try await shortcuts.labels().filter { identifiers.contains($0.id) }.map {
                LabelEntity(id: $0.id, name: $0.name)
            }
        }

        package func suggestedEntities() async throws -> [LabelEntity] {
            try await shortcuts.labels().map { LabelEntity(id: $0.id, name: $0.name) }
        }
    }
}

/// Finds tasks by status, Avenue, Project, label and due date, for a
/// shortcut to go through. Refused while Lifr's lock is on.
package struct FindTasksIntent: AppIntent {
    package static let title: LocalizedStringResource = "Find Tasks"
    package static let description = IntentDescription(
        """
        Finds tasks in Lifr by status, Avenue, Project, label or due date. With no status set, it finds \
        open tasks.
        """)

    @Parameter(title: "Status")
    package var status: StatusOption?
    @Parameter(title: "Avenue")
    package var avenue: AvenueEntity?
    @Parameter(title: "Project")
    package var project: ProjectEntity?
    @Parameter(title: "Label")
    package var label: LabelEntity?
    @Parameter(title: "Due By")
    package var dueBy: Date?

    @AppDependency private var shortcuts: Shortcuts

    package init() {}

    package static var parameterSummary: some ParameterSummary {
        Summary("Find tasks") {
            \.$status
            \.$avenue
            \.$project
            \.$label
            \.$dueBy
        }
    }

    package func perform() async throws -> some IntentResult & ReturnsValue<[TaskEntity]> & ProvidesDialog {
        let found = try await shortcuts.find(
            Shortcuts.Filter(
                status: status?.status, avenue: avenue?.id, project: project?.id, label: label?.id, dueBy: dueBy))
        let dialog: IntentDialog =
            switch found.count {
            case 0: "No tasks match."
            case 1: "Found 1 task."
            default: "Found \(found.count) tasks."
            }
        return .result(value: found.map(TaskEntity.init), dialog: dialog)
    }
}

/// Gives a task's fields as text: status, due date, Avenue, Project,
/// labels and description. Refused while Lifr's lock is on.
package struct GetTaskDetailsIntent: AppIntent {
    package static let title: LocalizedStringResource = "Get Task Details"
    package static let description = IntentDescription(
        "Gives a task's status, due date, Avenue, Project, labels and description as text.")

    @Parameter(title: "Task", requestValueDialog: "Which task?")
    package var task: TaskEntity

    @AppDependency private var shortcuts: Shortcuts

    package init() {}

    package static var parameterSummary: some ParameterSummary {
        Summary("Get the details of \(\.$task)")
    }

    package func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await shortcuts.details(of: task.id).text)
    }
}

/// A list Lifr can open to.
package enum ListOption: String, AppEnum {
    case today, upcoming, calendar

    package static let typeDisplayRepresentation: TypeDisplayRepresentation = "List"
    package static let caseDisplayRepresentations: [ListOption: DisplayRepresentation] = [
        .today: "Today", .upcoming: "Upcoming", .calendar: "Calendar",
    ]

    /// The link to the list; the calendar opens on the day `now` falls on.
    func link(now: Date, timeZone: TimeZone) -> DeepLink {
        switch self {
        case .today: .today
        case .upcoming: .upcoming
        case .calendar: .calendar(CalendarDate(now, in: timeZone))
        }
    }
}

/// Opens Lifr to Today, Upcoming or the calendar.
package struct OpenListIntent: AppIntent {
    package static let title: LocalizedStringResource = "Open a List"
    package static let description = IntentDescription("Opens Lifr to Today, Upcoming or the calendar.")
    package static let openAppWhenRun = true

    @Parameter(title: "List", requestValueDialog: "Which list?")
    package var list: ListOption

    package init() {}

    package static var parameterSummary: some ParameterSummary {
        Summary("Open \(\.$list)")
    }

    @MainActor
    package func perform() async throws -> some IntentResult & OpensIntent {
        var open = OpenURLIntent()
        if let url = list.link(now: Date(), timeZone: .current).url { open.url = url }
        return .result(opensIntent: open)
    }
}

/// Opens an Avenue in Lifr.
package struct OpenAvenueIntent: AppIntent {
    package static let title: LocalizedStringResource = "Open an Avenue"
    package static let description = IntentDescription("Opens an Avenue in Lifr.")
    package static let openAppWhenRun = true

    @Parameter(title: "Avenue", requestValueDialog: "Which Avenue?")
    package var avenue: AvenueEntity

    package init() {}

    package static var parameterSummary: some ParameterSummary {
        Summary("Open \(\.$avenue)")
    }

    @MainActor
    package func perform() async throws -> some IntentResult & OpensIntent {
        var open = OpenURLIntent()
        if let url = DeepLink.avenue(avenue.id).url { open.url = url }
        return .result(opensIntent: open)
    }
}

/// Copies a plan prompt, for Siri: said aloud, a prompt is too long to hear
/// and has nowhere to go, so it's put on the clipboard to paste.
package struct CopyPlanPromptIntent: AppIntent {
    package static let title: LocalizedStringResource = "Copy a Plan Prompt"
    package static let description = IntentDescription(
        """
        Copies a prompt asking another chat assistant for a plan Lifr can read. It names your Avenues, \
        Projects and labels, but no tasks. Paste the reply into a Lifr chat with /paste.
        """)

    @Parameter(title: "Goal", requestValueDialog: "What do you want to plan?")
    package var goal: String

    @AppDependency private var shortcuts: Shortcuts
    private var pasteboard: PasteboardClient { .liveValue }

    package init() {}

    package static var parameterSummary: some ParameterSummary {
        Summary("Copy a prompt to plan \(\.$goal)")
    }

    package func perform() async throws -> some IntentResult & ProvidesDialog {
        let prompt = try await shortcuts.planPrompt(goal: goal)
        await pasteboard.copy(prompt)
        return .result(
            dialog: """
                Copied a prompt to plan \(goal). Paste it into a chat assistant, then paste its reply \
                into a Lifr chat with /paste.
                """)
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
        AppShortcut(
            intent: OpenListIntent(),
            phrases: ["Open \(\.$list) in \(.applicationName)", "Show a list in \(.applicationName)"],
            shortTitle: "Open a List", systemImageName: "list.bullet")
        AppShortcut(
            intent: CopyPlanPromptIntent(),
            phrases: ["Copy a plan prompt in \(.applicationName)", "Plan something with \(.applicationName)"],
            shortTitle: "Copy a Plan Prompt", systemImageName: "doc.on.clipboard")
    }
}

/// Limits Lifr, its badge and its widgets to the chosen Avenues while a
/// Focus is on. The system runs it again with none chosen when the Focus
/// goes off, which shows every Avenue again.
package struct LifrFocusFilter: SetFocusFilterIntent {
    package static let title: LocalizedStringResource = "Show Only Some Avenues"
    package static let description: IntentDescription? = IntentDescription(
        "While this Focus is on, Lifr, its badge and its widgets show only the Avenues you pick.")

    @Parameter(title: "Avenues", default: [])
    package var avenues: [AvenueEntity]

    package init() {}

    package var displayRepresentation: DisplayRepresentation {
        avenues.isEmpty
            ? DisplayRepresentation(title: "Every Avenue")
            : DisplayRepresentation(title: "\(avenues.map(\.name).formatted(.list(type: .and)))")
    }

    package func perform() async throws -> some IntentResult {
        UserDefaults.standard.set(FocusAvenues(Set(avenues.map(\.id))).rawValue, forKey: FocusAvenues.key)
        return .result()
    }
}
