import LifrAPI
import LifrCore
import LifrUI
import SwiftUI

#if os(macOS)
    import AppKit
#else
    import UIKit
#endif

/// The plan to add, as rows the person can edit or remove before adding.
/// Every edit is checked against their records, and Add adds exactly what
/// the card shows.
struct DraftCard: View {
    @Binding var plan: Plan
    let problem: String?
    let canAdd: Bool
    let add: () -> Void
    let discard: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("Draft").font(.headline)
                Spacer()
                Menu {
                    Button("Copy as TOML", systemImage: "doc.on.doc") { copy(PlanFile.write(plan)) }
                } label: {
                    Label("More", systemImage: "ellipsis.circle").labelStyle(.iconOnly)
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
            }
            ForEach(plan.avenues.indices, id: \.self) { index in
                let avenue = $plan.avenues.element(index, or: .init(key: "", name: ""))
                Row(symbol: "square.stack.3d.up") {
                    TextField("Avenue name", text: avenue.name)
                    TextField("KEY", text: avenue.key).frame(maxWidth: 100)
                } remove: {
                    plan.avenues.remove(at: index)
                }
            }
            ForEach(plan.labels.indices, id: \.self) { index in
                Row(symbol: "tag") {
                    TextField("Label", text: $plan.labels.element(index, or: .init(name: "")).name)
                } remove: {
                    plan.labels.remove(at: index)
                }
            }
            ForEach(plan.projects.indices, id: \.self) { index in
                let project = $plan.projects.element(index, or: .init(name: "", avenue: ""))
                Row(symbol: "flag") {
                    TextField("Project name", text: project.name)
                } remove: {
                    plan.projects.remove(at: index)
                }
                HStack {
                    TextField("Avenue", text: project.avenue)
                    TextField("Due", text: project.due.orEmpty)
                }
                .font(.caption)
                .padding(.leading, 28)
                Tasks(tasks: project.tasks, showsPlace: false).padding(.leading)
            }
            Tasks(tasks: $plan.tasks, showsPlace: true)
            if let problem {
                Label(problem, systemImage: "exclamationmark.triangle")
                    .font(.caption)
                    .foregroundStyle(.red)
            }
            HStack {
                Button("Discard", role: .destructive, action: discard)
                Spacer()
                Button("Add", action: add)
                    .buttonStyle(.borderedProminent)
                    .disabled(!canAdd)
            }
        }
        .textFieldStyle(.roundedBorder)
        .padding()
        .background(.fill.quaternary, in: .rect(cornerRadius: 12))
    }

    private func copy(_ text: String) {
        #if os(macOS)
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
        #else
            UIPasteboard.general.string = text
        #endif
    }
}

/// Tasks with their Sub tasks, each editable.
private struct Tasks: View {
    @Binding var tasks: [Plan.TaskDraft]
    /// Whether a task says where it goes, which tasks in a new Project don't.
    let showsPlace: Bool

    var body: some View {
        ForEach(tasks.indices, id: \.self) { index in
            let task = $tasks.element(index, or: .init(title: ""))
            Row(symbol: "circle") {
                TextField("Title", text: task.title)
            } remove: {
                tasks.remove(at: index)
            }
            HStack {
                Picker("Priority", selection: task.priority) {
                    ForEach(Priority.allCases, id: \.self) { priority in
                        Text(priority.title ?? String(localized: "No priority")).tag(priority)
                    }
                }
                .labelsHidden()
                .fixedSize()
                TextField("Due", text: task.due.orEmpty)
                if showsPlace {
                    if let parent = task.wrappedValue.parent {
                        Text("Under \(parent)").font(.caption).foregroundStyle(.secondary)
                    } else {
                        TextField("Avenue or KEY/Project", text: task.place.orEmpty)
                    }
                }
            }
            .font(.caption)
            .padding(.leading, 28)
            ForEach(task.wrappedValue.subtasks.indices, id: \.self) { subtask in
                Row(symbol: "circle") {
                    TextField("Sub task", text: task.subtasks.element(subtask, or: .init(title: "")).title)
                } remove: {
                    tasks[index].subtasks.remove(at: subtask)
                }
                .padding(.leading, 28)
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
