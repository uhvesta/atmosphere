import ChatUI
import LifrAPI
import LifrCore
import LifrUI
import SwiftUI

/// What the plan adds, to read before adding: each row with what it means,
/// such as "Sat, Nov 7" for "saturday" or "Home" for "HOME". Edit opens the
/// plan's fields; Add adds exactly what the card shows.
struct DraftCard: View {
    let plan: Plan
    let meanings: DraftMeanings
    let problem: String?
    let canAdd: Bool
    let add: () -> Void
    let discard: () -> Void
    let edit: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                Label("Draft", systemImage: "doc.badge.plus")
                    .font(.headline)
                    .accessibilityAddTraits(.isHeader)
                Spacer()
                Text(plan.taskCount == 1 ? "1 task" : "\(plan.taskCount) tasks")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier(ModelText.identifier)
            }
            VStack(alignment: .leading, spacing: 8) {
                ForEach(Array(plan.avenues.enumerated()), id: \.offset) { _, avenue in
                    Line(symbol: "square.stack.3d.up", title: Text("New Avenue \(avenue.name)"), detail: avenue.key)
                }
                ForEach(Array(plan.labels.enumerated()), id: \.offset) { _, label in
                    Line(symbol: "tag", title: Text("New label \(label.name)"), detail: nil)
                }
                ForEach(Array(plan.projects.enumerated()), id: \.offset) { _, project in
                    Line(
                        symbol: "flag", title: Text("New Project \(project.name)"),
                        detail: meanings.details(due: project.due, place: project.avenue))
                    TaskLines(tasks: project.tasks, showsPlace: false, meanings: meanings)
                        .padding(.leading, 28)
                }
                TaskLines(tasks: plan.tasks, showsPlace: true, meanings: meanings)
            }
            if let problem {
                Label(problem, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .foregroundStyle(.red)
                    .accessibilityIdentifier(ModelText.identifier)
            }
            HStack {
                Button("Discard", role: .destructive, action: discard)
                Spacer()
                Button("Edit", action: edit)
                    .buttonStyle(.bordered)
                Button("Add", action: add)
                    .buttonStyle(.borderedProminent)
                    .disabled(!canAdd)
            }
        }
        .padding()
        .background(.background, in: .rect(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(.separator))
    }
}

/// Tasks as lines, each with its Sub tasks under it.
private struct TaskLines: View {
    let tasks: [Plan.TaskDraft]
    let showsPlace: Bool
    let meanings: DraftMeanings

    var body: some View {
        ForEach(Array(tasks.enumerated()), id: \.offset) { _, task in
            Line(
                symbol: "circle", title: Text(task.title),
                detail: meanings.details(
                    due: task.due,
                    place: showsPlace ? task.parent.map { String(localized: "Under \($0)") } ?? task.place : nil,
                    priority: task.priority, resolvesPlace: task.parent == nil))
            ForEach(Array(task.subtasks.enumerated()), id: \.offset) { _, subtask in
                Line(symbol: "circle", title: Text(subtask.title), detail: nil, small: true)
                    .padding(.leading, 28)
            }
        }
    }
}

/// One thing the plan adds: a symbol, its title and what's set on it.
private struct Line: View {
    let symbol: String
    let title: Text
    let detail: String?
    var small = false

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: symbol)
                .imageScale(small ? .small : .medium)
                .foregroundStyle(.secondary)
                .frame(width: 20)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                title.font(small ? .callout : .body)
                if let detail {
                    Text(detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier(ModelText.identifier)
    }
}

/// What a draft's fields mean in the person's records: the day a due date
/// is, and the Avenue and Project a place names.
struct DraftMeanings {
    let workspace: Workspace
    let today: CalendarDate?
    let locale: Locale

    /// The Avenue, and Project, `text` names, when it names one.
    func place(_ text: String?) -> String? {
        guard let text, !text.isEmpty else { return nil }
        let parts = text.split(separator: "/", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
        guard
            let avenue = workspace.avenues.values.first(where: {
                $0.key.rawValue.caseInsensitiveCompare(parts.first ?? "") == .orderedSame
            })
        else { return nil }
        guard parts.count == 2 else { return avenue.name }
        return "\(avenue.name) › \(parts[1])"
    }

    /// The day `text` is, such as "Sat, Nov 7" for "saturday".
    func day(_ text: String?) -> String? {
        guard let text, !text.isEmpty, let today, let date = CalendarDate.resolving(text, today: today) else {
            return nil
        }
        return date.noonUTC.formatted(
            Date.FormatStyle(locale: locale, calendar: CalendarDate.utcCalendar, timeZone: .gmt)
                .weekday(.abbreviated).month(.abbreviated).day())
    }

    /// When it's due, where it goes and how urgent it is, as one line; what
    /// Lifr can't read is shown as written.
    func details(
        due: String?, place: String?, priority: Priority = .none, resolvesPlace: Bool = true
    ) -> String? {
        var parts: [String] = []
        if let due, !due.isEmpty { parts.append(day(due) ?? due) }
        if let place, !place.isEmpty { parts.append(resolvesPlace ? self.place(place) ?? place : place) }
        if let title = priority.title { parts.append(title) }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}

/// The plan's fields, to change before adding: every edit is checked against
/// the person's records, with what each field means shown under it.
struct DraftEditor: View {
    @Binding var plan: Plan
    let meanings: DraftMeanings
    let problem: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                ForEach(plan.avenues.indices, id: \.self) { index in
                    let avenue = $plan.avenues.element(index, or: .init(key: "", name: ""))
                    Row(symbol: "square.stack.3d.up") {
                        TextField("Avenue name", text: avenue.name).accessibilityLabel("Avenue name")
                        TextField("KEY", text: avenue.key).accessibilityLabel("Key").frame(maxWidth: 100)
                    } remove: {
                        plan.avenues.remove(at: index)
                    }
                }
                ForEach(plan.labels.indices, id: \.self) { index in
                    Row(symbol: "tag") {
                        TextField("Label", text: $plan.labels.element(index, or: .init(name: "")).name)
                            .accessibilityLabel("Label")
                    } remove: {
                        plan.labels.remove(at: index)
                    }
                }
                ForEach(plan.projects.indices, id: \.self) { index in
                    let project = $plan.projects.element(index, or: .init(name: "", avenue: ""))
                    Row(symbol: "flag") {
                        TextField("Project name", text: project.name).accessibilityLabel("Project name")
                    } remove: {
                        plan.projects.remove(at: index)
                    }
                    HStack(alignment: .top) {
                        Meaning(meanings.place(project.wrappedValue.avenue)) {
                            TextField("Avenue", text: project.avenue).accessibilityLabel("Avenue")
                        }
                        Meaning(meanings.day(project.wrappedValue.due)) {
                            TextField("Due", text: project.due.orEmpty).accessibilityLabel("Due")
                        }
                    }
                    .font(.caption)
                    .padding(.leading, 28)
                    Tasks(tasks: project.tasks, showsPlace: false, place: meanings.place, day: meanings.day)
                        .padding(.leading)
                }
                Tasks(tasks: $plan.tasks, showsPlace: true, place: meanings.place, day: meanings.day)
                if let problem {
                    Label(problem, systemImage: "exclamationmark.triangle")
                        .font(.caption)
                        .foregroundStyle(.red)
                }
            }
            .textFieldStyle(.roundedBorder)
            .padding()
        }
        .toolbar {
            ToolbarItem(placement: .secondaryAction) {
                Button("Copy as TOML", systemImage: "doc.on.doc") { copyToClipboard(PlanFile.write(plan)) }
            }
        }
    }
}

/// Tasks with their Sub tasks, each editable.
private struct Tasks: View {
    @Binding var tasks: [Plan.TaskDraft]
    /// Whether a task says where it goes, which tasks in a new Project don't.
    let showsPlace: Bool
    let place: (String?) -> String?
    let day: (String?) -> String?

    var body: some View {
        ForEach(tasks.indices, id: \.self) { index in
            let task = $tasks.element(index, or: .init(title: ""))
            Row(symbol: "circle") {
                TextField("Title", text: task.title).accessibilityLabel("Title")
            } remove: {
                tasks.remove(at: index)
            }
            HStack(alignment: .top) {
                Picker("Priority", selection: task.priority) {
                    ForEach(Priority.allCases, id: \.self) { priority in
                        Text(priority.title ?? String(localized: "No priority")).tag(priority)
                    }
                }
                .labelsHidden()
                .fixedSize()
                Meaning(day(task.wrappedValue.due)) {
                    TextField("Due", text: task.due.orEmpty).accessibilityLabel("Due")
                }
                if showsPlace {
                    if let parent = task.wrappedValue.parent {
                        Text("Under \(parent)").font(.caption).foregroundStyle(.secondary)
                    } else {
                        Meaning(place(task.wrappedValue.place)) {
                            TextField("Avenue or KEY/Project", text: task.place.orEmpty).accessibilityLabel(
                                "Avenue or KEY/Project")
                        }
                    }
                }
            }
            .font(.caption)
            .padding(.leading, 28)
            ForEach(task.wrappedValue.subtasks.indices, id: \.self) { subtask in
                Row(symbol: "circle") {
                    TextField("Sub task", text: task.subtasks.element(subtask, or: .init(title: "")).title)
                        .accessibilityLabel("Sub task")
                } remove: {
                    tasks[index].subtasks.remove(at: subtask)
                }
                .padding(.leading, 28)
            }
        }
    }
}

/// A field, with what it means under it when Lifr can tell.
private struct Meaning<Field: View>: View {
    let meaning: String?
    @ViewBuilder let field: Field

    init(_ meaning: String?, @ViewBuilder field: () -> Field) {
        self.meaning = meaning
        self.field = field()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            field
            if let meaning {
                Text(meaning).foregroundStyle(.secondary)
            }
        }
    }
}

/// A draft row: its symbol, its fields, and a button that removes it.
private struct Row<Fields: View>: View {
    let symbol: String
    @ViewBuilder let fields: Fields
    let remove: () -> Void

    var body: some View {
        HStack {
            Image(systemName: symbol).foregroundStyle(.secondary).frame(width: 20)
            fields
            Button("Remove", systemImage: "xmark.circle.fill", role: .destructive, action: remove)
                .labelStyle(.iconOnly)
                .buttonStyle(.borderless)
                .foregroundStyle(.secondary)
        }
    }
}

extension Binding {
    /// The element at `index`. Reads give `fallback` once it's removed, and
    /// writes are dropped, while SwiftUI catches up with the shorter list.
    fileprivate func element<Element>(_ index: Int, or fallback: Element) -> Binding<Element>
    where Value == [Element] {
        Binding<Element>(
            get: { wrappedValue.indices.contains(index) ? wrappedValue[index] : fallback },
            set: { if wrappedValue.indices.contains(index) { wrappedValue[index] = $0 } })
    }
}

extension Binding<String?> {
    /// The text, with empty meaning none.
    fileprivate var orEmpty: Binding<String> {
        Binding<String>(
            get: { wrappedValue ?? "" },
            set: { wrappedValue = $0.isEmpty ? nil : $0 })
    }
}
