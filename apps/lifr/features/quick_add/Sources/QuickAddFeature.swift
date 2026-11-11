import ComposableArchitecture
import Foundation
import LifrCore
import LifrRemindersClient
import LifrStoreClient
import LifrTaskDetail

/// Adds a task or a Project in a few taps. A task needs an Avenue (the
/// last one used) and optionally a Project, a due day and a priority; a
/// Project needs its Avenue. A task's title can say the rest, as in
/// "Call dentist tomorrow 3pm !high #health": each recognised part shows
/// under the field and wins over the form. Sub tasks are added from their task: opened
/// with `parent`, or in Sub task mode after adding the task, Quick Add
/// stays open and each title becomes a Sub task of it.
@Reducer
package struct QuickAddFeature {
    /// What Quick Add makes.
    package enum Kind: String, CaseIterable, Hashable, Sendable {
        case task, project

        package var title: String {
            switch self {
            case .task: String(localized: "Task")
            case .project: String(localized: "Project")
            }
        }
    }

    /// When the new task is due.
    package enum Due: String, CaseIterable, Hashable, Sendable {
        case none, today, tomorrow, pick

        package var title: String {
            switch self {
            case .none: String(localized: "None")
            case .today: String(localized: "Today")
            case .tomorrow: String(localized: "Tomorrow")
            case .pick: String(localized: "Pick")
            }
        }
    }

    package struct Choice: Identifiable, Hashable, Sendable {
        package var id: UUID
        package var name: String

        package init(id: UUID, name: String) {
            self.id = id
            self.name = name
        }
    }

    @ObservableState
    package struct State: Equatable {
        package var kind = Kind.task
        package var title = ""
        package var avenueID: UUID?
        package var projectID: UUID?
        package var due = Due.none
        package var pickedDate: Date
        /// Whether the due day has a time, for a title that gives none.
        package var addsTime = false
        package var pickedTime: Date
        package var priority = Priority.none
        package var subtaskMode = false
        /// The task Sub tasks are added to, once Sub task mode added it.
        package var parent: Choice?
        /// Titles added under `parent` so far.
        package var addedSubtasks: [String] = []
        package var error: String?
        package var avenues: [Choice]
        package var projects: [UUID: [Choice]]
        /// Words in the title the person chose to keep as text.
        package var ignored: Set<String> = []
        /// What the title is read against: its labels, Avenues and Projects.
        package var workspace: Workspace
        package var today: CalendarDate
        /// Whether "11/5" is Nov 5 where the person is.
        package var monthFirst: Bool
        /// The Avenue last added to, on this device.
        @Shared(.appStorage("lifrQuickAddLastAvenue")) package var lastAvenue = ""
        @Shared(.defaultReminder) package var defaultReminder

        package init(workspace: Workspace, now: Date, timeZone: TimeZone, locale: Locale = .current) {
            pickedDate = now
            // The next whole hour.
            let hours = (now.timeIntervalSinceReferenceDate / 3600).rounded(.down)
            pickedTime = Date(timeIntervalSinceReferenceDate: hours * 3600 + 3600)
            self.workspace = workspace
            today = CalendarDate(now, in: timeZone)
            let order = DateFormatter.dateFormat(fromTemplate: "Md", options: 0, locale: locale) ?? "M/d"
            let month = order.firstIndex(of: "M") ?? order.startIndex
            monthFirst = month <= (order.firstIndex(of: "d") ?? order.endIndex)
            avenues = workspace.avenues.values
                .filter { $0.archivedAt == nil }
                .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                .map { Choice(id: $0.id, name: $0.name) }
            projects = Dictionary(
                grouping: workspace.projects.values
                    .filter { $0.status == .active }
                    .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) },
                by: \.avenueID
            ).mapValues { $0.map { Choice(id: $0.id, name: $0.name) } }
            let last = UUID(uuidString: lastAvenue)
            avenueID = avenues.first { $0.id == last }?.id ?? avenues.first?.id
        }

        /// Projects in the picked Avenue.
        package var avenueProjects: [Choice] {
            avenueID.flatMap { projects[$0] } ?? []
        }

        /// The title as read, for a new task; nil for a Project or a Sub task.
        package var entry: QuickEntry? {
            guard parent == nil, kind == .task else { return nil }
            return QuickEntry(title, today: today, in: workspace, ignoring: ignored, monthFirst: monthFirst)
        }

        /// The title the new item gets.
        package var finalTitle: String {
            (entry?.title ?? title).trimmingCharacters(in: .whitespacesAndNewlines)
        }

        package var canAdd: Bool {
            !finalTitle.isEmpty && (parent != nil || avenueID != nil || entry?.avenueID != nil)
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case addTapped
        /// Text pasted to add a task for each of its lines, each read as
        /// the title is.
        case linesPasted(String)
        /// A recognised part tapped: read its words as plain text.
        case partIgnored(String)
        case doneTapped
        case cancelTapped
        case added(Result<Saved, any Error>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Snapshot)
        }
    }

    @Dependency(\.store) private var store
    @Dependency(\.dismiss) private var dismiss
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
            .onChange(of: \.title) { _, title in
                Reduce { state, _ in
                    // A cleared field reads its next title afresh.
                    if title.isEmpty { state.ignored = [] }
                    return .none
                }
            }
            .onChange(of: \.avenueID) { _, _ in
                Reduce { state, _ in
                    if let project = state.projectID, !state.avenueProjects.contains(where: { $0.id == project }) {
                        state.projectID = nil
                    }
                    return .none
                }
            }
        Reduce { state, action in
            switch action {
            case .addTapped:
                guard state.canAdd else { return .none }
                let title = state.finalTitle
                state.error = nil
                if state.parent == nil, state.kind == .project, let avenueID = state.avenueID {
                    return .run { [store] send in
                        await send(.added(Result { try await store.createProject(title, in: avenueID) }))
                    }
                }
                var new = Workspace.NewTask(title: title)
                if let parent = state.parent {
                    new.parentID = parent.id
                } else {
                    new = task(title, entry: state.entry, in: state)
                }
                return .run { [store, new] send in
                    await send(.added(Result { try await store.createTask(new) }))
                }

            case .linesPasted(let text):
                guard state.parent == nil, state.kind == .task else { return .none }
                let tasks = Self.lines(text).compactMap { line -> Workspace.NewTask? in
                    let entry = QuickEntry(line, today: state.today, in: state.workspace, monthFirst: state.monthFirst)
                    let title = entry.title.trimmingCharacters(in: .whitespacesAndNewlines)
                    guard !title.isEmpty, entry.avenueID != nil || state.avenueID != nil else { return nil }
                    return task(title, entry: entry, in: state)
                }
                guard !tasks.isEmpty else {
                    state.error = String(localized: "The clipboard has no lines to add. Copy a list, one task a line.")
                    return .none
                }
                state.error = nil
                // Each line is a task of its own, not a parent of the next.
                state.subtaskMode = false
                return .run { [store, tasks] send in
                    await send(.added(Result { try await store.createTasks(tasks) }))
                }

            case .added(.success(let saved)):
                let created = saved.change.previous.first { $0.value == nil }.flatMap { saved.workspace.tasks[$0.key] }
                if state.parent != nil {
                    state.addedSubtasks.append(state.finalTitle)
                    state.title = ""
                    return .send(.delegate(.saved(saved.snapshot)))
                }
                state.workspace = saved.workspace
                if let avenueID = created?.avenueID ?? state.avenueID {
                    state.$lastAvenue.withLock { $0 = avenueID.uuidString }
                }
                if state.kind == .task, state.subtaskMode, let created {
                    state.parent = Choice(id: created.id, name: created.title)
                    state.title = ""
                    state.ignored = []
                    return .send(.delegate(.saved(saved.snapshot)))
                }
                return .run { [dismiss] send in
                    await send(.delegate(.saved(saved.snapshot)))
                    await dismiss()
                }

            case .added(.failure(let error)):
                state.error = (error as? LifrError)?.description ?? error.localizedDescription
                return .none

            case .partIgnored(let key):
                state.ignored.insert(key)
                return .none

            case .doneTapped, .cancelTapped:
                return .run { [dismiss] _ in await dismiss() }

            case .binding, .delegate:
                return .none
            }
        }
    }

    /// A new top-level task called `title`: what `entry` read from its
    /// line, then what's picked below the field.
    private func task(_ title: String, entry: QuickEntry?, in state: State) -> Workspace.NewTask {
        var new = Workspace.NewTask(title: title)
        new.avenueID = entry?.avenueID ?? state.avenueID
        new.projectID = entry?.avenueID == nil ? state.projectID : entry?.projectID
        new.priority = entry?.priority ?? state.priority
        new.dueDate = entry?.dueDate ?? dueDate(state)
        new.dueTime =
            entry?.dueTime
            ?? (state.addsTime && new.dueDate != nil ? TimeOfDay(state.pickedTime, in: timeZone) : nil)
        new.labelIDs = entry?.labelIDs ?? []
        if new.dueDate != nil, let trigger = state.defaultReminder.trigger(dueTime: new.dueTime) {
            new.reminders = [trigger]
        }
        return new
    }

    /// The lines of pasted text, without list markers such as "- ", "* ",
    /// "1. " or "[ ] ", and without blank ones.
    static func lines(_ text: String) -> [String] {
        text.components(separatedBy: .newlines).compactMap { line in
            var rest = Substring(line.trimmingCharacters(in: .whitespaces))
            for marker in ["- ", "* ", "• ", "+ "] where rest.hasPrefix(marker) {
                rest = rest.dropFirst(marker.count)
                break
            }
            if let dot = rest.firstIndex(where: { $0 == "." || $0 == ")" }), dot > rest.startIndex,
                rest[..<dot].allSatisfy(\.isNumber), rest[rest.index(after: dot)...].first == " "
            {
                rest = rest[rest.index(dot, offsetBy: 2)...]
            }
            for box in ["[ ] ", "[x] ", "[X] "] where rest.hasPrefix(box) {
                rest = rest.dropFirst(box.count)
                break
            }
            let title = rest.trimmingCharacters(in: .whitespaces)
            // A marker with nothing after it, such as a lone "-".
            return title.allSatisfy({ "-*•+".contains($0) }) ? nil : title
        }
    }

    private func dueDate(_ state: State) -> CalendarDate? {
        switch state.due {
        case .none: nil
        case .today: CalendarDate(now, in: timeZone)
        case .tomorrow: CalendarDate(now, in: timeZone).adding(days: 1)
        case .pick: CalendarDate(state.pickedDate, in: timeZone)
        }
    }
}
