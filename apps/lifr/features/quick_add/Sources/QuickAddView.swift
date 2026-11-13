import ComposableArchitecture
import LifrCore
import LifrUI
import SwiftUI

package struct QuickAddView: View {
    @Bindable package var store: StoreOf<QuickAddFeature>
    @FocusState private var titleFocused: Bool

    package init(store: StoreOf<QuickAddFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            if store.parent == nil {
                Section {
                    Picker("Add", selection: $store.kind) {
                        ForEach(QuickAddFeature.Kind.allCases, id: \.self) { Text($0.title).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                }
            }
            Section {
                TextField(fieldTitle, text: $store.title, prompt: Text(fieldTitle))
                    .titleField()
                    .focused($titleFocused)
                    .onSubmit { store.send(.addTapped) }
                    .submitLabel(.done)
                if store.parent == nil, store.kind == .task {
                    // The system's button reads the clipboard without
                    // asking, as the person pressed it.
                    HStack {
                        Text("Add a task for each line copied")
                            .foregroundStyle(.secondary)
                        Spacer()
                        PasteButton(payloadType: String.self) { strings in
                            store.send(.linesPasted(strings.joined(separator: "\n")))
                        }
                        .labelStyle(.titleAndIcon)
                        .accessibilityHint("Adds a task for each line copied")
                    }
                }
            } footer: {
                let entry = store.entry
                if entry?.parts.isEmpty == false || store.error != nil {
                    VStack(alignment: .leading, spacing: 8) {
                        if let entry, !entry.parts.isEmpty {
                            // The line as typed, the words read tinted, so
                            // it's clear which ones the chips stand for.
                            // The chips say the same to VoiceOver.
                            Text(Self.highlighted(store.title, entry.parts))
                                .accessibilityHidden(true)
                            // The title as it will be saved, without them.
                            Text("Adds “\(store.finalTitle)”")
                            PartChips(entry: entry, workspace: store.workspace) { store.send(.partIgnored($0)) }
                        }
                        if let error = store.error {
                            Text(error).foregroundStyle(.red)
                        }
                    }
                }
            }
            if let parent = store.parent {
                Section {
                    ForEach(Array(store.addedSubtasks.enumerated()), id: \.offset) { _, title in
                        Label(title, systemImage: "checkmark.circle")
                    }
                } header: {
                    Text("Sub tasks of \(parent.name)")
                }
            } else {
                switch store.kind {
                case .task: fields
                case .project: avenueField
                }
            }
        }
        .navigationTitle(navigationTitle)
        #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
        #endif
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                if store.parent == nil {
                    Button("Cancel") { store.send(.cancelTapped) }
                }
            }
            ToolbarItem(placement: .confirmationAction) {
                if store.parent == nil {
                    Button("Add") { store.send(.addTapped) }.disabled(!store.canAdd)
                } else {
                    Button("Done") { store.send(.doneTapped) }
                }
            }
        }
        .onAppear { titleFocused = true }
    }

    /// `line` with the words of `parts` tinted and bold.
    static func highlighted(_ line: String, _ parts: [QuickEntry.Part]) -> AttributedString {
        var text = AttributedString(line)
        let characters = text.characters
        for part in parts where part.range.lowerBound >= 0 && part.range.upperBound <= characters.count {
            let start = characters.index(characters.startIndex, offsetBy: part.range.lowerBound)
            let end = characters.index(characters.startIndex, offsetBy: part.range.upperBound)
            text[start..<end].foregroundColor = .accentColor
            text[start..<end].inlinePresentationIntent = .stronglyEmphasized
        }
        return text
    }

    private var fieldTitle: LocalizedStringKey {
        if store.parent != nil { return "Sub task title" }
        return store.kind == .project ? "Name" : "Title"
    }

    private var navigationTitle: LocalizedStringKey {
        if store.parent != nil { return "Add Sub tasks" }
        switch store.kind {
        case .task: return "New Task"
        case .project: return "New Project"
        }
    }

    /// The Avenue a new Project goes in.
    private var avenueField: some View {
        Section {
            Picker("Avenue", selection: $store.avenueID) {
                ForEach(store.avenues) { Text($0.name).tag(Optional($0.id)) }
            }
        } footer: {
            if store.avenues.isEmpty {
                Text("Add an Avenue first, on the Avenues tab.")
            }
        }
    }

    private var hasDueDay: Bool { store.entry?.dueDate != nil || store.due != .none }

    /// A field the title sets shows as a chip instead of its picker, so the
    /// two never disagree; removing the chip brings the picker back.
    @ViewBuilder
    private var fields: some View {
        if store.entry?.avenueID == nil {
            Section {
                Picker("Avenue", selection: $store.avenueID) {
                    ForEach(store.avenues) { Text($0.name).tag(Optional($0.id)) }
                }
                if !store.avenueProjects.isEmpty {
                    Picker("Project", selection: $store.projectID) {
                        Text("None").tag(UUID?.none)
                        ForEach(store.avenueProjects) { Text($0.name).tag(Optional($0.id)) }
                    }
                }
            } footer: {
                if store.avenues.isEmpty {
                    Text("Add an Avenue first, on the Avenues tab.")
                }
            }
        }
        if store.entry?.dueDate == nil || store.entry?.priority == nil || hasDueDay {
            Section {
                if store.entry?.dueDate == nil {
                    Picker("Due", selection: $store.due) {
                        ForEach(QuickAddFeature.Due.allCases, id: \.self) { Text($0.title).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    if store.due == .pick {
                        DatePicker("Due", selection: $store.pickedDate, displayedComponents: .date)
                    }
                }
                if hasDueDay, store.entry?.dueTime == nil {
                    Toggle("Add a time", isOn: $store.addsTime)
                    if store.addsTime {
                        DatePicker("Time", selection: $store.pickedTime, displayedComponents: .hourAndMinute)
                    }
                }
                if store.entry?.priority == nil {
                    Picker("Priority", selection: $store.priority) {
                        ForEach(Priority.allCases, id: \.self) {
                            Text($0.title ?? String(localized: "None")).tag($0)
                        }
                    }
                }
            }
        }
        Section {
            Toggle("Add Sub tasks next", isOn: $store.subtaskMode)
        } footer: {
            Text("Adds this task, then keeps Quick Add open for its Sub tasks.")
        }
    }
}

/// The parts of the title Quick Add recognised, each tapped to keep its
/// words as text instead.
private struct PartChips: View {
    let entry: QuickEntry
    let workspace: Workspace
    let ignore: (String) -> Void
    @Environment(\.locale) private var locale

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(Array(entry.parts.enumerated()), id: \.offset) { _, part in
                    Button {
                        ignore(part.key)
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: symbol(part.kind))
                            Text(title(part.kind))
                            Image(systemName: "xmark")
                                .font(.caption2.weight(.bold))
                                .foregroundStyle(.secondary)
                        }
                        .font(.caption)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(.tint.opacity(0.15), in: Capsule())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(title(part.kind))
                    .accessibilityHint("Keeps “\(part.text)” in the title instead")
                }
            }
        }
    }

    private func symbol(_ kind: QuickEntry.Part.Kind) -> String {
        switch kind {
        case .due: "calendar"
        case .time: "clock"
        case .priority: "flag"
        case .label: "tag"
        case .avenue, .project: "folder"
        }
    }

    private func title(_ kind: QuickEntry.Part.Kind) -> String {
        switch kind {
        case .due(let date): date.readable(locale)
        case .time(let time): time.formatted(locale)
        case .priority(let priority): priority.title ?? String(localized: "No priority")
        case .label(let id): "#" + (workspace.labels[id]?.name ?? "")
        case .avenue(let id): workspace.avenues[id]?.name ?? ""
        case .project(let avenue, let project):
            "\(workspace.avenues[avenue]?.name ?? "") › \(workspace.projects[project]?.name ?? "")"
        }
    }
}

#Preview {
    NavigationStack {
        QuickAddView(
            store: Store(
                initialState: QuickAddFeature.State(
                    workspace: .sample(now: Date(), timeZone: .current), now: Date(), timeZone: .current)
            ) {
                QuickAddFeature()
            })
    }
}
