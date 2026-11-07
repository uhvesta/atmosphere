import AssistantKit
import SwiftUI

/// A field of a record, before and after, in words: what a proposal would
/// change, or what a change in the log did.
public struct ChangedField: Hashable, Codable, Sendable {
    public var record: String
    public var field: String
    public var before: String?
    public var after: String?

    public init(record: String, field: String, before: String?, after: String?) {
        self.record = record
        self.field = field
        self.before = before
        self.after = after
    }

    /// The changes, with those to the same record in a row together, in
    /// the order they come.
    public static func byRecord(_ changes: [ChangedField]) -> [(record: String, fields: [ChangedField])] {
        var groups: [(record: String, fields: [ChangedField])] = []
        for change in changes {
            if let last = groups.indices.last, groups[last].record == change.record {
                groups[last].fields.append(change)
            } else {
                groups.append((change.record, [change]))
            }
        }
        return groups
    }
}

/// A field before → after, such as "Status: To Do → Done".
public struct FieldLine: View {
    let change: ChangedField

    public init(_ change: ChangedField) {
        self.change = change
    }

    public var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            Text("\(change.field):")
                .foregroundStyle(.secondary)
            if let before = change.before {
                Text(before)
                    .strikethrough()
                    .foregroundStyle(.secondary)
                Image(systemName: "arrow.right")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("to")
            }
            Text(change.after ?? String(localized: "none"))
        }
        .font(.footnote)
    }
}

/// What a reply asks to change, field by field, before → after, with
/// Apply, Edit and Decline until the person decides; then what they
/// decided, with Undo while what was applied can be.
public struct ProposalCard: View {
    let summary: String
    let changes: [ChangedField]
    let decision: ProposalDecision?
    let canUndo: Bool
    let isBusy: Bool
    let apply: () -> Void
    let edit: () -> Void
    let decline: () -> Void
    let undo: () -> Void

    public init(
        summary: String, changes: [ChangedField], decision: ProposalDecision?, canUndo: Bool, isBusy: Bool,
        apply: @escaping () -> Void, edit: @escaping () -> Void, decline: @escaping () -> Void,
        undo: @escaping () -> Void
    ) {
        self.summary = summary
        self.changes = changes
        self.decision = decision
        self.canUndo = canUndo
        self.isBusy = isBusy
        self.apply = apply
        self.edit = edit
        self.decline = decline
        self.undo = undo
    }

    public var body: some View {
        let groups = ChangedField.byRecord(changes)
        VStack(alignment: .leading, spacing: 8) {
            Text(summary)
                .font(.subheadline.weight(.semibold))
            ForEach(groups, id: \.record) { group in
                VStack(alignment: .leading, spacing: 2) {
                    // One record is named in the summary already.
                    if groups.count > 1 {
                        Text(group.record)
                            .font(.footnote.weight(.medium))
                    }
                    ForEach(group.fields, id: \.self) { FieldLine($0) }
                }
            }
            if decision == nil {
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
                    Label(status, systemImage: decision == .applied ? "checkmark.circle" : "xmark.circle")
                        .foregroundStyle(.secondary)
                    if decision == .applied, canUndo {
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

    private var status: String {
        switch (decision, canUndo) {
        case (.applied?, true): String(localized: "Applied")
        case (.applied?, false): String(localized: "Applied, then undone")
        case (.declined?, _): String(localized: "Declined")
        case (nil, _): ""
        }
    }
}

/// The changes a reply proposed, each to keep or leave out, and what each
/// sets, which the app lays out. Done checks them against the records as
/// they are now.
public struct ProposalEditor<Fields: View>: View {
    let titles: [String]
    @Binding var kept: [Bool]
    let problem: String?
    let isBusy: Bool
    let done: () -> Void
    let fields: (Int) -> Fields
    @Environment(\.dismiss) private var dismiss

    /// One section for each of `titles`, with `fields(index)` while it's
    /// kept.
    public init(
        titles: [String], kept: Binding<[Bool]>, problem: String?, isBusy: Bool, done: @escaping () -> Void,
        @ViewBuilder fields: @escaping (Int) -> Fields
    ) {
        self.titles = titles
        _kept = kept
        self.problem = problem
        self.isBusy = isBusy
        self.done = done
        self.fields = fields
    }

    public var body: some View {
        Form {
            if let problem {
                Label(problem, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.orange)
            }
            ForEach(titles.indices, id: \.self) { index in
                Section {
                    Toggle(titles[index], isOn: $kept[index])
                    if kept.indices.contains(index), kept[index] {
                        fields(index)
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
}

/// A change in the log: what it did, who made it and when, and each field
/// before → after.
public struct ActivityEntryRow: View {
    let summary: String
    let author: String
    let at: Date
    let changes: [ChangedField]
    let isReverted: Bool

    public init(summary: String, author: String, at: Date, changes: [ChangedField], isReverted: Bool) {
        self.summary = summary
        self.author = author
        self.at = at
        self.changes = changes
        self.isReverted = isReverted
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                Text(summary).font(.headline)
                if isReverted {
                    Text("Reverted")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Text(at, format: .dateTime.month().day().hour().minute())
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Text(author).font(.subheadline).foregroundStyle(.secondary)
            ForEach(Array(changes.enumerated()), id: \.offset) { _, change in
                Text(Self.describe(change, showsRecord: changes.count > 1))
                    .font(.caption)
            }
        }
        .accessibilityElement(children: .combine)
    }

    /// "Priority: None → High", with the record when there's more than one.
    private static func describe(_ change: ChangedField, showsRecord: Bool) -> String {
        let name = showsRecord ? "\(change.record) · \(change.field)" : change.field
        return "\(name): \(change.before ?? "—") → \(change.after ?? "—")"
    }
}
