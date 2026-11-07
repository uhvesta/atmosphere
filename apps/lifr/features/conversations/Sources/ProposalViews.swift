import AssistantKit
import ChatUI
import LifrAssistantClient
import LifrCore
import LifrUI
import SwiftUI

extension AssistantKit.Proposal where Action == AssistantAction, Step == JournalStep {
    /// Fields the change bumps by itself, which say nothing new.
    private static var bookkeeping: Set<Field> { [.author, .closedAt, .completedAt, .editedAt] }

    /// What it would change, in words, for its card.
    func card(
        in workspace: Workspace, canUndo: Bool, isBusy: Bool, apply: @escaping () -> Void,
        edit: @escaping () -> Void, decline: @escaping () -> Void, undo: @escaping () -> Void
    ) -> ProposalCard {
        let words = JournalWords(workspace) { _ in "" }
        return ProposalCard(
            summary: words.summary(of: steps, reverts: reverts),
            changes: words.fields(of: steps, hiding: Self.bookkeeping), decision: decision, canUndo: canUndo,
            isBusy: isBusy, apply: apply, edit: edit, decline: decline, undo: undo)
    }
}

/// The changes a reply proposed, each to keep or leave out, and what each
/// sets. Done checks them against the records as they are now.
struct ChangesEditor: View {
    @Binding var edit: ChatFeature.ProposalEdit
    /// The records the changes are to, for their names.
    let workspace: Workspace
    let isBusy: Bool
    let done: () -> Void

    var body: some View {
        ProposalEditor(
            titles: edit.actions.map(title), kept: $edit.kept, problem: edit.problem, isBusy: isBusy, done: done
        ) { index in
            ActionFields(action: $edit.actions[index])
        }
    }

    private func title(of action: AssistantAction) -> String {
        switch action {
        case .task(let change):
            (try? workspace.task(code: change.code)).map { String(localized: "Change “\($0.title)”") }
                ?? String(localized: "Change \(change.code)")
        case .project(let change): String(localized: "Change \(change.project)")
        case .comment(let code, _): String(localized: "Comment on \(code)")
        case .revert(let entries):
            entries.count == 1
                ? String(localized: "Revert a change") : String(localized: "Revert \(entries.count) changes")
        }
    }
}

/// What one change sets, to edit. A field left empty stays as it is.
private struct ActionFields: View {
    @Binding var action: AssistantAction

    var body: some View {
        switch action {
        case .task(let change):
            let task = Binding {
                change
            } set: {
                action = .task($0)
            }
            LabeledContent("Title") {
                TextField("Title", text: task.title.orEmpty, prompt: Text("Unchanged"))
                    .labelsHidden()
            }
            Picker("Status", selection: task.status) {
                Text("Unchanged").tag(String?.none)
                ForEach(Self.taskStatuses, id: \.key) { status in
                    Text(status.value.title).tag(String?.some(status.key))
                }
            }
            Picker("Priority", selection: task.priority) {
                Text("Unchanged").tag(String?.none)
                ForEach(Priority.allCases, id: \.self) { priority in
                    Text(priority.title ?? String(localized: "None")).tag(String?.some(priority.rawValue))
                }
            }
            LabeledContent("Due") {
                TextField("Due", text: task.due.orEmpty, prompt: Text("Such as tomorrow, or none"))
                    .labelsHidden()
            }
            LabeledContent("Move to") {
                TextField("Move to", text: task.place.orEmpty, prompt: Text("Avenue key, or KEY/Project"))
                    .labelsHidden()
            }
        case .project(let change):
            let project = Binding {
                change
            } set: {
                action = .project($0)
            }
            LabeledContent("Name") {
                TextField("Name", text: project.name.orEmpty, prompt: Text("Unchanged"))
                    .labelsHidden()
            }
            Picker("Status", selection: project.status) {
                Text("Unchanged").tag(String?.none)
                Text("Active").tag(String?.some("active"))
                Text("Completed").tag(String?.some("completed"))
                Text("Archived").tag(String?.some("archived"))
            }
            LabeledContent("Due") {
                TextField("Due", text: project.due.orEmpty, prompt: Text("Such as Friday, or none"))
                    .labelsHidden()
            }
            TextField("Details", text: project.details.orEmpty, axis: .vertical)
        case .comment(let code, let text):
            TextField(
                "Comment",
                text: Binding {
                    text
                } set: {
                    action = .comment(code: code, text: $0)
                }, axis: .vertical)
        case .revert:
            EmptyView()
        }
    }

    /// The statuses the assistant can set, by the name it uses.
    private static let taskStatuses: [(key: String, value: TaskStatus)] = [
        ("todo", .todo), ("in_progress", .inProgress), ("done", .done), ("wont_do", .wontDo),
    ]
}
