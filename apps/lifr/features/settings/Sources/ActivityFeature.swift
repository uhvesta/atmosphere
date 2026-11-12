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
        case .savedFilter: String(localized: "Saved Filters")
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
    /// The entry is still in the undo window. Older ones are archived.
    package var canRevert = true

    /// A field of a record, before and after, in words.
    package typealias Change = JournalWords.FieldWords
}

/// The Activity log: every change, newest first, with who made it and what
/// it changed, and a way to put any of them back.
@Reducer
package struct ActivityFeature {
    @ObservableState
    package struct State: Equatable {
        package var workspace: Workspace
        /// The latest save shown, so an earlier one's late reply is ignored.
        package var saves = SaveOrder()
        package var author: ActivityAuthor?
        package var kind: RecordKind?
        package var period = ActivityPeriod.all
        /// The entries too old to undo, once the person asks for them.
        package var archived: [ArchivedEntry]?
        @Presents package var confirm: ConfirmationDialogState<Confirm>?
        @Presents package var alert: AlertState<Never>?

        /// Shows `snapshot`'s records unless later ones are shown already.
        package mutating func show(_ snapshot: Snapshot) {
            guard saves.isLatest(snapshot) else { return }
            workspace = snapshot.workspace
        }

        package init(workspace: Workspace) {
            self.workspace = workspace
        }

        /// The entries that match the filters, newest first, then the
        /// archived ones once they're loaded.
        package func rows(now: Date, calendar: Calendar) -> [ActivityRow] {
            let start = period.start(now: now, calendar: calendar)
            let older = archived ?? []
            let reverted = Set(workspace.journal.flatMap(\.reverts) + older.flatMap(\.reverts))
            func matches(_ actor: Actor, _ at: Date, _ kinds: [RecordKind]) -> Bool {
                if let author, ActivityAuthor(actor) != author { return false }
                if let kind, !kinds.contains(kind) { return false }
                if let start, at < start { return false }
                return true
            }
            let words = JournalWords(workspace) { actor in self.author(actor) }
            let recent = workspace.journal.reversed()
                .filter { matches($0.actor, $0.at, $0.steps.map(\.record.kind)) }
                .map { row($0, isReverted: reverted.contains($0.id), words: words) }
            let archivedRows = older.reversed()
                .filter { matches($0.actor, $0.at, $0.steps.map(\.record.kind)) }
                .map { entry in
                    ActivityRow(
                        id: entry.id, summary: words.summary(of: entry.steps, reverts: entry.reverts.count),
                        author: author(entry.actor), at: entry.at, changes: words.fields(of: entry.steps),
                        isReverted: reverted.contains(entry.id), isRevert: !entry.reverts.isEmpty, canRevert: false)
                }
            return recent + archivedRows
        }

        private func row(_ entry: JournalEntry, isReverted: Bool, words: JournalWords) -> ActivityRow {
            ActivityRow(
                id: entry.id, summary: words.summary(of: entry.steps, reverts: entry.reverts.count),
                author: author(entry.actor), at: entry.at, changes: words.fields(of: entry.steps),
                isReverted: isReverted, isRevert: !entry.reverts.isEmpty)
        }

        private func author(_ actor: Actor) -> String {
            let place = ActivityAuthor(actor).title
            switch actor.kind {
            case .agent where actor.surface != .assistant:
                return String(localized: "\(actor.client ?? String(localized: "An agent")) · \(place)")
            default: return place
            }
        }
    }

    package enum Confirm: Equatable, Sendable {
        case revert([UUID])
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case revertTapped(UUID)
        case revertSinceTapped(UUID)
        case olderTapped
        case olderLoaded(Result<[ArchivedEntry], any Error>)
        case confirm(PresentationAction<Confirm>)
        case saved(Result<Saved, any Error>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Snapshot)
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

            case .olderTapped:
                return .run { [store] send in
                    await send(.olderLoaded(Result { try await store.archivedJournal() }))
                }

            case .olderLoaded(.success(let entries)):
                state.archived = entries
                return .none

            case .olderLoaded(.failure(let error)):
                state.alert = AlertState {
                    TextState("Couldn't show older changes")
                } message: {
                    TextState("\(error.localizedDescription) Try again in a moment.")
                }
                return .none

            case .confirm(.presented(.revert(let ids))):
                return .run { [store] send in
                    await send(.saved(Result { try await store.revert(entries: ids) }))
                }

            case .saved(.success(let saved)):
                let isLatest = saved.number >= state.saves.latest
                state.show(saved.snapshot)
                if !saved.change.skipped.isEmpty {
                    state.alert = AlertState {
                        TextState("Some fields were kept")
                    } message: {
                        TextState(Self.skipped(saved.change.skipped.count))
                    }
                }
                return isLatest ? .send(.delegate(.saved(saved.snapshot))) : .none

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
