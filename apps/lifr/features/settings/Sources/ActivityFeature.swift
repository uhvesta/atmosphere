import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrUI

/// Who made a change, as the Activity log groups them.
package enum ActivityAuthor: Hashable, Sendable {
    case surface(Actor.Surface)
    /// Lifr's own fixes, such as reopening a parent after a sync.
    case lifr

    package init(_ actor: Actor) {
        self = actor.kind == .system ? .lifr : .surface(actor.surface)
    }

    package static let all: [ActivityAuthor] = Actor.Surface.allCases.map(Self.surface) + [.lifr]

    package var title: String {
        switch self {
        case .surface(.iPhone): "iPhone"
        case .surface(.iPad): "iPad"
        case .surface(.mac): String(localized: "Mac")
        case .surface(.cli): String(localized: "Command line")
        case .surface(.mcp): "MCP"
        case .surface(.assistant): String(localized: "Apple Intelligence")
        case .surface(.shortcuts): String(localized: "Shortcuts")
        case .lifr: String(localized: "Lifr")
        }
    }
}

/// How far back the Activity log goes.
package enum ActivityPeriod: String, Hashable, Sendable, CaseIterable {
    case today, week, month, all

    package var title: String {
        switch self {
        case .today: String(localized: "Today")
        case .week: String(localized: "Past 7 days")
        case .month: String(localized: "Past 30 days")
        case .all: String(localized: "All time")
        }
    }

    func start(now: Date, calendar: Calendar) -> Date? {
        switch self {
        case .today: calendar.startOfDay(for: now)
        case .week: now.addingTimeInterval(-7 * 24 * 60 * 60)
        case .month: now.addingTimeInterval(-30 * 24 * 60 * 60)
        case .all: nil
        }
    }
}

extension RecordKind {
    package var title: String {
        switch self {
        case .avenue: String(localized: "Avenues")
        case .project: String(localized: "Projects")
        case .task: String(localized: "Tasks")
        case .label: String(localized: "Labels")
        case .comment: String(localized: "Comments")
        case .reminder: String(localized: "Reminders")
        case .settings: String(localized: "Settings")
        }
    }

    fileprivate var noun: String {
        switch self {
        case .avenue: String(localized: "Avenue")
        case .project: String(localized: "Project")
        case .task: "task"
        case .label: "label"
        case .comment: "comment"
        case .reminder: "reminder"
        case .settings: "settings"
        }
    }
}

/// One journal entry as the Activity log shows it.
package struct ActivityRow: Identifiable, Hashable, Sendable {
    package var id: UUID
    package var summary: String
    /// Who, and where: "Claude Code · MCP", "iPhone".
    package var author: String
    package var at: Date
    package var changes: [Change]
    /// A later entry reverted this one.
    package var isReverted: Bool
    /// This entry reverts others.
    package var isRevert: Bool

    /// A field of a record, before and after, in words.
    package struct Change: Hashable, Sendable {
        package var record: String
        package var field: String
        package var before: String?
        package var after: String?
    }
}

/// The Activity log: every change, newest first, with who made it and what
/// it changed, and a way to put any of them back.
@Reducer
package struct ActivityFeature {
    @ObservableState
    package struct State: Equatable {
        package var workspace: Workspace
        package var author: ActivityAuthor?
        package var kind: RecordKind?
        package var period = ActivityPeriod.all
        @Presents package var confirm: ConfirmationDialogState<Confirm>?
        @Presents package var alert: AlertState<Never>?

        package init(workspace: Workspace) {
            self.workspace = workspace
        }

        /// The entries that match the filters, newest first.
        package func rows(now: Date, calendar: Calendar) -> [ActivityRow] {
            let start = period.start(now: now, calendar: calendar)
            let reverted = Set(workspace.journal.flatMap(\.reverts))
            return workspace.journal.reversed()
                .filter { entry in
                    if let author, ActivityAuthor(entry.actor) != author { return false }
                    if let kind, !entry.steps.contains(where: { $0.record.kind == kind }) { return false }
                    if let start, entry.at < start { return false }
                    return true
                }
                .map { row($0, isReverted: reverted.contains($0.id)) }
        }

        private func row(_ entry: JournalEntry, isReverted: Bool) -> ActivityRow {
            ActivityRow(
                id: entry.id, summary: summary(entry), author: author(entry.actor), at: entry.at,
                changes: entry.steps.flatMap { step in
                    let record = name(step)
                    return step.fields.filter { !Self.hidden.contains($0.field) }.map { change in
                        ActivityRow.Change(
                            record: record, field: change.field.title, before: change.before.map(text),
                            after: change.after.map(text))
                    }
                },
                isReverted: isReverted, isRevert: !entry.reverts.isEmpty)
        }

        /// Fields people don't need to see: the order, and the task a
        /// comment or reminder is on, which the row already names.
        private static let hidden: Set<Field> = [.sortOrder, .task]

        private func summary(_ entry: JournalEntry) -> String {
            if !entry.reverts.isEmpty {
                return entry.reverts.count == 1
                    ? String(localized: "Reverted a change")
                    : String(localized: "Reverted \(entry.reverts.count) changes")
            }
            guard let first = entry.steps.first else { return String(localized: "No change") }
            var text = "\(verb(first)) \(first.record.kind.noun)"
            if first.record.kind != .settings { text += " “\(title(first))”" }
            return entry.steps.count == 1 ? text : text + " and \(entry.steps.count - 1) more"
        }

        private func verb(_ step: JournalStep) -> String {
            switch (step.before, step.after) {
            case (nil, _): return String(localized: "Added")
            case (_, nil): return String(localized: "Removed")
            case (.task(let old)?, .task(let new)?):
                if old.deletedAt == nil, new.deletedAt != nil { return String(localized: "Trashed") }
                if old.deletedAt != nil, new.deletedAt == nil { return String(localized: "Restored") }
                return String(localized: "Changed")
            case (.comment(let old)?, .comment(let new)?) where old.deletedAt == nil && new.deletedAt != nil:
                return String(localized: "Deleted")
            default: return String(localized: "Changed")
            }
        }

        private func title(_ step: JournalStep) -> String {
            switch step.after ?? step.before {
            case .avenue(let avenue): avenue.name
            case .project(let project): project.name
            case .task(let task): task.title
            case .label(let label): label.name
            case .comment(let comment): workspace.tasks[comment.taskID]?.title ?? "a task"
            case .reminder(let reminder): workspace.tasks[reminder.taskID]?.title ?? "a task"
            case .settings, nil: String(localized: "Settings")
            }
        }

        private func name(_ step: JournalStep) -> String {
            step.record.kind == .settings ? String(localized: "Settings") : title(step)
        }

        private func author(_ actor: Actor) -> String {
            let place = ActivityAuthor(actor).title
            switch actor.kind {
            case .agent where actor.surface != .assistant:
                return String(localized: "\(actor.client ?? String(localized: "An agent")) · \(place)")
            default: return place
            }
        }

        /// A value as people read it: records by name, times as dates.
        private func text(_ value: FieldValue) -> String {
            switch value {
            case .text(let text): text
            case .number(let number): String(number)
            case .day(let day): day.description
            case .time(let time): time.description
            case .moment(let date): date.formatted(date: .abbreviated, time: .shortened)
            case .taskStatus(let status): status.title
            case .projectStatus(let status): status.rawValue.capitalized
            case .priority(let priority): priority.title ?? String(localized: "None")
            case .access(let access): access.title
            case .record(let kind, let id): name(of: kind, id)
            case .records(let kind, let ids): ids.map { name(of: kind, $0) }.sorted().joined(separator: ", ")
            case .actor(let actor): author(actor)
            case .trigger(.at(let date)): date.formatted(date: .abbreviated, time: .shortened)
            case .trigger(.beforeDue(let seconds)):
                Duration.seconds(seconds).formatted(.units(allowed: [.days, .hours, .minutes], width: .wide))
                    + " before due"
            }
        }

        private func name(of kind: RecordKind, _ id: UUID) -> String {
            let name: String? =
                switch kind {
                case .avenue: workspace.avenues[id]?.name
                case .project: workspace.projects[id]?.name
                case .task: workspace.tasks[id]?.title
                case .label: workspace.labels[id]?.name
                case .comment, .reminder, .settings: nil
                }
            return name ?? "something removed"
        }
    }

    package enum Confirm: Equatable, Sendable {
        case revert([UUID])
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case revertTapped(UUID)
        case revertSinceTapped(UUID)
        case confirm(PresentationAction<Confirm>)
        case saved(Result<Saved, any Error>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Workspace)
        }
    }

    @Dependency(\.store) private var store
    @Dependency(\.date.now) private var now

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .revertTapped(let id):
                state.confirm = confirmation([id], in: state.workspace, title: String(localized: "Revert this change?"))
                return .none

            case .revertSinceTapped(let id):
                guard let entry = state.workspace.journal.first(where: { $0.id == id }) else { return .none }
                let ids = state.workspace.entries(since: entry.at).map(\.id)
                state.confirm = confirmation(
                    ids, in: state.workspace, title: String(localized: "Revert \(ids.count) changes, back to this one?")
                )
                return .none

            case .confirm(.presented(.revert(let ids))):
                return .run { [store] send in
                    await send(.saved(Result { try await store.revert(entries: ids) }))
                }

            case .saved(.success(let saved)):
                state.workspace = saved.workspace
                if !saved.change.skipped.isEmpty {
                    state.alert = AlertState {
                        TextState("Some fields were kept")
                    } message: {
                        TextState(Self.skipped(saved.change.skipped.count))
                    }
                }
                return .send(.delegate(.saved(saved.workspace)))

            case .saved(.failure(let error)):
                state.alert = AlertState {
                    TextState("Couldn't revert")
                } message: {
                    TextState("\(error.localizedDescription) Nothing changed.")
                }
                return .none

            case .binding, .confirm, .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$confirm, action: \.confirm)
        .ifLet(\.$alert, action: \.alert)
    }

    /// Asks before reverting, saying up front what it would leave as it is.
    private func confirmation(_ ids: [UUID], in workspace: Workspace, title: String)
        -> ConfirmationDialogState<Confirm>
    {
        var copy = workspace
        let preview = try? copy.revert(ids, in: Context(actor: .system(device: "", surface: .iPhone), now: now))
        let kept = preview?.skipped.count ?? 0
        return ConfirmationDialogState(titleVisibility: .visible) {
            TextState(title)
        } actions: {
            ButtonState(role: .destructive, action: .revert(ids)) { TextState("Revert") }
            ButtonState(role: .cancel) { TextState("Cancel") }
        } message: {
            kept == 0
                ? TextState("The revert is a change of its own, so you can revert it too.")
                : TextState("The revert is a change of its own, so you can revert it too. \(Self.skipped(kept))")
        }
    }

    private static func skipped(_ count: Int) -> String {
        count == 1
            ? String(localized: "One field was changed again since, so it stays as it is now.")
            : String(localized: "\(count) fields were changed again since, so they stay as they are now.")
    }
}
