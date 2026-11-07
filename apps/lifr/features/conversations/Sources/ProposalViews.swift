import LifrAssistantClient
import LifrCore
import LifrUI
import SwiftUI

/// What a reply asks to change, field by field, before → after, with
/// Apply, Edit and Decline until the person decides.
struct ProposalCard: View {
    let proposal: Proposal
    let workspace: Workspace
    let canUndo: Bool
    let isBusy: Bool
    let apply: () -> Void
    let edit: () -> Void
    let decline: () -> Void
    let undo: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(words.summary(of: proposal.steps, reverts: proposal.reverts))
                .font(.subheadline.weight(.semibold))
            ForEach(groups, id: \.record) { group in
                VStack(alignment: .leading, spacing: 2) {
                    // One record is named in the summary already.
                    if groups.count > 1 {
                        Text(group.record)
                            .font(.footnote.weight(.medium))
                    }
                    ForEach(group.fields, id: \.self) { field in
                        FieldLine(field: field)
                    }
                }
            }
            if proposal.decision == nil {
                HStack {
                    Button("Decline", role: .destructive, action: decline)
                    Spacer()
                    Button("Edit", action: edit)
                    Button("Apply", action: apply)
                        .buttonStyle(.borderedProminent)
                }
                .disabled(isBusy)
                .font(.subheadline)
            } else {
                HStack {
                    Label(status, systemImage: proposal.decision == .applied ? "checkmark.circle" : "xmark.circle")
                        .foregroundStyle(.secondary)
                    if proposal.decision == .applied, canUndo {
                        Spacer()
                        Button("Undo", action: undo)
                    }
                }
                .font(.caption)
            }
        }
        .padding(12)
        .frame(maxWidth: 520, alignment: .leading)
        .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: 12))
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
    }

    private var words: JournalWords { JournalWords(workspace) { _ in "" } }

    /// Fields the change bumps by itself, which say nothing new.
    private static let bookkeeping: Set<Field> = [.author, .closedAt, .completedAt, .editedAt]

    private var groups: [(record: String, fields: [JournalWords.FieldWords])] {
        var groups: [(record: String, fields: [JournalWords.FieldWords])] = []
        for field in words.fields(of: proposal.steps, hiding: Self.bookkeeping) {
            if let last = groups.indices.last, groups[last].record == field.record {
                groups[last].fields.append(field)
            } else {
                groups.append((field.record, [field]))
            }
        }
        return groups
    }

    private var status: String {
        switch (proposal.decision, canUndo) {
        case (.applied?, true): String(localized: "Applied")
        case (.applied?, false): String(localized: "Applied, then undone")
        case (.declined?, _): String(localized: "Declined")
        case (nil, _): ""
        }
    }
}

/// A field before → after, such as "Status: To Do → Done".
private struct FieldLine: View {
    let field: JournalWords.FieldWords

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            Text("\(field.field):")
                .foregroundStyle(.secondary)
            if let before = field.before {
                Text(before)
                    .strikethrough()
                    .foregroundStyle(.secondary)
                Image(systemName: "arrow.right")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("to")
            }
            Text(field.after ?? String(localized: "none"))
        }
        .font(.footnote)
    }
}

/// The changes a reply proposed, each to keep or leave out, and what each
/// sets. Done checks them against the records as they are now.
struct ProposalEditor: View {
    @Binding var edit: ChatFeature.ProposalEdit
    /// The records the changes are to, for their names.
    let workspace: Workspace
    let isBusy: Bool
    let done: () -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Form {
            if let problem = edit.problem {
                Label(problem, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.orange)
            }
            ForEach(edit.actions.indices, id: \.self) { index in
                Section {
                    Toggle(title(of: edit.actions[index]), isOn: $edit.kept[index])
                    if edit.kept[index] {
                        ActionFields(action: $edit.actions[index])
                    }
                }
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Edit Changes")
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Done", action: done)
                    .disabled(isBusy)
            }
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

extension Binding where Value == String? {
    /// The text, with empty text as `nil`.
    fileprivate var orEmpty: Binding<String> {
        Binding<String>(
            get: { wrappedValue ?? "" },
            set: { wrappedValue = $0.isEmpty ? nil : $0 })
    }
}
