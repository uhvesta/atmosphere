import ComposableArchitecture
import Foundation
import LifrCore
import LifrSyncClient

/// The records two devices changed in ways that couldn't merge. For each,
/// the person picks which device's value to keep, field by field, or
/// whether to keep a record one device deleted and the other edited.
@Reducer
package struct SyncReviewFeature {
    @ObservableState
    package struct State: Equatable {
        package var items: [Review.Item]
        /// For record names, such as a task's Avenue.
        package var workspace: Workspace
        /// This device, as the choices name it, such as "This iPhone".
        package var thisDevice: String
        /// What the person picked. A choice not yet made keeps this
        /// device's value.
        package var choices: Review.Choices = [:]
        package var isResolving = false
        /// Why writing the choices failed, until the next try.
        package var error: String?

        package init(items: [Review.Item], workspace: Workspace, thisDevice: String) {
            self.items = items
            self.workspace = workspace
            self.thisDevice = thisDevice
        }

        package func choice(_ record: RecordID, _ field: Field) -> SyncSide {
            choices[record]?[field] ?? .mine
        }

        /// Every question answered, unanswered ones with this device's side.
        package var answers: Review.Choices {
            Dictionary(
                uniqueKeysWithValues: items.map { item in
                    let fields: [Field] =
                        switch item.conflict.kind {
                        case .fields(let fields): fields
                        case .removed: [.deletedAt]
                        }
                    return (item.id, Dictionary(uniqueKeysWithValues: fields.map { ($0, choice(item.id, $0)) }))
                })
        }

        package var rows: [SyncReviewRow] {
            items.map { SyncReviewRow($0, in: workspace) }
        }
    }

    package enum Action {
        case chose(RecordID, Field, SyncSide)
        /// Keeps what was chosen and syncs.
        case doneTapped
        /// Closes the sheet and leaves the records held for later.
        case laterTapped
        case resolved(SyncState)
        /// New conflicts arrived while the sheet was up.
        case show([Review.Item], Workspace)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            /// The choices were written, so the records changed.
            case resolved
            case finished
        }
    }

    @Dependency(\.sync) private var sync

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .chose(let record, let field, let side):
                state.choices[record, default: [:]][field] = side
                return .none

            case .doneTapped:
                guard !state.isResolving else { return .none }
                state.isResolving = true
                state.error = nil
                let answers = state.answers
                return .run { [sync] send in await send(.resolved(await sync.resolve(answers))) }

            case .laterTapped:
                return .send(.delegate(.finished))

            case .resolved(let result):
                state.isResolving = false
                if case .failed(let error) = result.status,
                    result.review.contains(where: { held in
                        state.items.contains { $0.id == held.id }
                    })
                {
                    state.error = error.description
                    return .none
                }
                guard !result.review.isEmpty else {
                    return .concatenate(.send(.delegate(.resolved)), .send(.delegate(.finished)))
                }
                // What's left arrived while the person chose.
                state.items = result.review
                state.choices = [:]
                return .send(.delegate(.resolved))

            case .show(let items, let workspace):
                state.workspace = workspace
                guard !state.isResolving else { return .none }
                state.items = items
                state.choices = state.choices.filter { record, _ in items.contains { $0.id == record } }
                return items.isEmpty ? .send(.delegate(.finished)) : .none

            case .delegate:
                return .none
            }
        }
    }
}

/// One record under review, in words.
package struct SyncReviewRow: Identifiable, Hashable, Sendable {
    package struct Question: Identifiable, Hashable, Sendable {
        package var field: Field
        package var title: String
        package var mine: String
        package var theirs: String
        package var id: Field { field }
    }

    package enum Kind: Hashable, Sendable {
        case fields([Question])
        /// One device deleted the record and the other changed it.
        case removed(by: SyncSide)
    }

    package var id: RecordID
    /// Such as "Task “Buy race gels”".
    package var title: String
    /// The other device's name.
    package var device: String
    package var kind: Kind

    package init(_ item: Review.Item, in workspace: Workspace) {
        let conflict = item.conflict
        id = item.id
        device = item.device
        let noun = Self.noun(conflict.record.kind)
        title = conflict.title.map { "\(noun) “\($0)”" } ?? noun
        switch conflict.kind {
        case .removed(let side):
            kind = .removed(by: side)
        case .fields(let fields):
            kind = .fields(
                fields.map { field in
                    let sides = conflict.sides(of: field)
                    func text(_ value: (Field, FieldValue?)) -> String {
                        let shown = value.1.map { Self.text($0, in: workspace) } ?? String(localized: "None")
                        return sides.count > 1 ? "\(value.0.title): \(shown)" : shown
                    }
                    return Question(
                        field: field,
                        title: sides.count > 1 ? String(localized: "Place") : field.title,
                        mine: sides.map { text(($0.field, $0.mine)) }.joined(separator: " · "),
                        theirs: sides.map { text(($0.field, $0.theirs)) }.joined(separator: " · "))
                })
        }
    }

    private static func noun(_ kind: RecordKind) -> String {
        switch kind {
        case .avenue: String(localized: "Avenue")
        case .project: String(localized: "Project")
        case .task: String(localized: "Task")
        case .label: String(localized: "Label")
        case .comment: String(localized: "Comment")
        case .reminder: String(localized: "Reminder")
        case .settings: String(localized: "Settings")
        }
    }

    /// A value as people read it: records by name, times as dates.
    private static func text(_ value: FieldValue, in workspace: Workspace) -> String {
        func name(_ kind: RecordKind, _ id: UUID) -> String {
            let name: String? =
                switch kind {
                case .avenue: workspace.avenues[id]?.name
                case .project: workspace.projects[id]?.name
                case .task: workspace.tasks[id]?.title
                case .label: workspace.labels[id]?.name
                case .comment, .reminder, .settings: nil
                }
            return name ?? String(localized: "something removed")
        }
        return switch value {
        case .text(let text): text
        case .number(let number): String(number)
        case .day(let day): day.description
        case .time(let time): time.description
        case .moment(let date): date.formatted(date: .abbreviated, time: .shortened)
        case .taskStatus(let status): status.title
        case .projectStatus(let status): status.rawValue.capitalized
        case .priority(let priority): priority.title ?? String(localized: "None")
        case .access(let access): access.title
        case .record(let kind, let id): name(kind, id)
        case .records(let kind, let ids): ids.map { name(kind, $0) }.sorted().joined(separator: ", ")
        case .actor(let actor): actor.client ?? actor.surface.rawValue
        case .trigger(.at(let date)): date.formatted(date: .abbreviated, time: .shortened)
        case .trigger(.beforeDue(let seconds)):
            Duration.seconds(seconds).formatted(.units(allowed: [.days, .hours, .minutes], width: .wide))
                + String(localized: " before due")
        }
    }
}
