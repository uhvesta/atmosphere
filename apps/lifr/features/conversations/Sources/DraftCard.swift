import ChatUI
import LifrAPI
import LifrCore
import LifrUI
import SwiftUI

/// What the plan adds, to read before adding: each row with what it means,
/// such as "Sat, Nov 7" for "saturday" or "Home" for "HOME". Edit opens the
/// plan's fields; Add, which VoiceOver calls Add Draft, adds exactly what the card shows.
struct DraftCard: View {
    let plan: Plan
    let meanings: DraftMeanings
    let problem: String?
    /// False while a reply is still writing the plan.
    let canChange: Bool
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
                // Tasks, not their Sub tasks, as the change says once it's added.
                let count = plan.projects.flatMap(\.tasks).count + plan.tasks.count
                Text("^[\(count) task](inflect: true)")
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
                        symbol: "folder", title: Text("New Project \(project.name)"),
                        detail: meanings.details(due: project.due, place: project.avenue))
                    TaskLines(tasks: project.tasks, showsPlace: false, meanings: meanings)
                        .padding(.leading, 28)
                }
                TaskLines(tasks: plan.tasks, showsPlace: true, meanings: meanings)
            }
            if closed > 0 {
                Label(
                    "^[\(closed) task](inflect: true) will be added already closed, as Done or Won't Do.",
                    systemImage: "checkmark.circle"
                )
                .font(.callout)
                .foregroundStyle(.orange)
                .accessibilityIdentifier(ModelText.identifier)
            }
            if let problem {
                Label(problem, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .foregroundStyle(.red)
                    .accessibilityIdentifier(ModelText.identifier)
            }
            HStack {
                Button("Discard", role: .destructive, action: discard)
                    .disabled(!canChange)
                Spacer()
                Button("Edit", action: edit)
                    .buttonStyle(.bordered)
                    .disabled(!canChange)
                // Named for VoiceOver apart from the composer's Add.
                Button("Add", action: add)
                    .buttonStyle(.borderedProminent)
                    .disabled(!canAdd)
                    .accessibilityLabel("Add Draft")
            }
        }
        .padding()
        .background(.background, in: .rect(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(.separator))
    }

    /// The tasks and Sub tasks the plan adds as Done or Won't Do, which
    /// another assistant's reply can carry over from the tasks it was sent.
    private var closed: Int {
        let tasks = plan.projects.flatMap(\.tasks) + plan.tasks
        return tasks.reduce(0) { count, task in
            count + (task.status.isClosed ? 1 : 0) + task.subtasks.count { $0.status.isClosed }
        }
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
                symbol: task.status.symbol, title: Text(task.title),
                detail: status(
                    task.status,
                    meanings.details(
                        due: task.due,
                        place: showsPlace ? task.parent.map { String(localized: "Under \($0)") } ?? task.place : nil,
                        priority: task.priority, resolvesPlace: task.parent == nil)))
            ForEach(Array(task.subtasks.enumerated()), id: \.offset) { _, subtask in
                Line(
                    symbol: subtask.status.symbol, title: Text(subtask.title), detail: status(subtask.status, nil),
                    small: true
                )
                .padding(.leading, 28)
            }
        }
    }
}

/// `detail` led by the status, unless it's To Do.
private func status(_ status: TaskStatus, _ detail: String?) -> String? {
    guard status != .todo else { return detail }
    guard let detail else { return status.title }
    return String(localized: "\(status.title) · \(detail)")
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

    /// A due date as VoiceOver reads it: "None" for an empty field, whose
    /// placeholder would otherwise be read as its value.
    static func spoken(_ due: String?) -> Text {
        guard let due, !due.isEmpty else { return Text("None") }
        return Text(due)
    }

    var body: some View {
        let avenues = PlaceChoice.all(in: meanings.workspace, adding: plan.avenues, projects: false)
        let places = PlaceChoice.all(in: meanings.workspace, adding: plan.avenues, projects: true)
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                ForEach(plan.avenues.indices, id: \.self) { index in
                    let avenue = $plan.avenues.element(index, or: .init(key: "", name: ""))
                    Row(symbol: "square.stack.3d.up", name: avenue.wrappedValue.name) {
                        TextField("Avenue name", text: avenue.name).accessibilityLabel("Avenue name")
                        TextField("KEY", text: avenue.key).accessibilityLabel("Key").frame(maxWidth: 100)
                    } remove: {
                        plan.avenues.remove(at: index)
                    }
                }
                ForEach(plan.labels.indices, id: \.self) { index in
                    Row(symbol: "tag", name: plan.labels.indices.contains(index) ? plan.labels[index].name : "") {
                        TextField("Label", text: $plan.labels.element(index, or: .init(name: "")).name)
                            .accessibilityLabel("Label")
                    } remove: {
                        plan.labels.remove(at: index)
                    }
                }
                ForEach(plan.projects.indices, id: \.self) { index in
                    let project = $plan.projects.element(index, or: .init(name: "", avenue: ""))
                    Row(symbol: "folder", name: project.wrappedValue.name) {
                        TextField("Project name", text: project.name).accessibilityLabel("Project name")
                    } remove: {
                        plan.projects.remove(at: index)
                    }
                    HStack(alignment: .top) {
                        PlacePicker(title: "Avenue", place: project.avenue.optional, choices: avenues, none: "Choose")
                            .labelsHidden()
                            .fixedSize()
                        Meaning(meanings.day(project.wrappedValue.due)) {
                            TextField("Due", text: project.due.orEmpty).accessibilityLabel("Due")
                                .accessibilityValue(Self.spoken(project.wrappedValue.due))
                        }
                    }
                    .font(.caption)
                    .padding(.leading, 28)
                    Tasks(tasks: project.tasks, places: nil, day: meanings.day)
                        .padding(.leading)
                }
                Tasks(tasks: $plan.tasks, places: places, day: meanings.day)
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
    /// Where a task can go; nil for tasks in a new Project, which go in it.
    let places: [PlaceChoice]?
    let day: (String?) -> String?

    var body: some View {
        ForEach(tasks.indices, id: \.self) { index in
            let task = $tasks.element(index, or: .init(title: ""))
            Row(symbol: "circle", name: task.wrappedValue.title) {
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
                        .accessibilityValue(DraftEditor.spoken(task.wrappedValue.due))
                }
                if let places {
                    if let parent = task.wrappedValue.parent {
                        Text("Under \(parent)").font(.caption).foregroundStyle(.secondary)
                    } else {
                        PlacePicker(title: "Place", place: task.place, choices: places, none: "Choose")
                            .labelsHidden()
                            .fixedSize()
                    }
                }
            }
            .font(.caption)
            .padding(.leading, 28)
            ForEach(task.wrappedValue.subtasks.indices, id: \.self) { subtask in
                let subtasks = task.wrappedValue.subtasks
                Row(symbol: "circle", name: subtasks.indices.contains(subtask) ? subtasks[subtask].title : "") {
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
    /// What the row holds, so Remove can say what it takes out.
    let name: String
    @ViewBuilder let fields: Fields
    let remove: () -> Void

    var body: some View {
        HStack {
            Image(systemName: symbol).foregroundStyle(.secondary).frame(width: 20).accessibilityHidden(true)
            fields
            Button("Remove", systemImage: "xmark.circle.fill", role: .destructive, action: remove)
                .labelStyle(.iconOnly)
                .buttonStyle(.borderless)
                .foregroundStyle(.secondary)
                .accessibilityLabel(name.isEmpty ? String(localized: "Remove") : String(localized: "Remove \(name)"))
                // Saying "Remove" alone works too; with several, Voice
                // Control numbers them.
                .accessibilityInputLabels(
                    name.isEmpty ? [Text("Remove")] : [Text("Remove \(name)"), Text("Remove"), Text("Delete")])
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
