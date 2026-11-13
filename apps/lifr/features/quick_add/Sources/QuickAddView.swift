import ComposableArchitecture
import LifrCore
import LifrUI
import SwiftUI

package struct QuickAddView: View {
    @Bindable package var store: StoreOf<QuickAddFeature>
    @FocusState private var titleFocused: Bool
    @Environment(\.locale) private var locale

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
                if #available(iOS 26, macOS 26, *) {
                    EntryField(
                        title: fieldTitle, line: store.title, parts: store.entry?.parts ?? [],
                        typed: { store.send(.titleTyped($0)) }
                    )
                    .focused($titleFocused)
                } else {
                    TextField(fieldTitle, text: $store.title, prompt: Text(fieldTitle))
                        .titleField()
                        .focused($titleFocused)
                        .onSubmit { store.send(.addTapped) }
                        .submitLabel(.done)
                }
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
                let readsWords = locale.language.languageCode == .english
                if entry?.parts.isEmpty == false || store.error != nil || !readsWords {
                    VStack(alignment: .leading, spacing: 8) {
                        if let entry, !entry.parts.isEmpty {
                            if #unavailable(iOS 26, macOS 26) {
                                // The line as typed, the words read tinted,
                                // where the field can't tint them itself.
                                // The chips say the same to VoiceOver.
                                Text(tinted(store.title, entry.parts)).accessibilityHidden(true)
                            }
                            // The title as it will be saved, without the
                            // words tinted.
                            Text("Adds “\(store.finalTitle)”")
                            PartChips(entry: entry, workspace: store.workspace) { store.send(.partIgnored($0)) }
                        }
                        if let error = store.error {
                            Text(error).foregroundStyle(.red)
                        }
                        if !readsWords {
                            // The line is read in English only, so say so
                            // rather than silently reading nothing.
                            Text(
                                """
                                Lifr reads dates, Avenues and priorities in the line only in English, such as \
                                “tomorrow #home !high”. Set them below instead.
                                """)
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
        case .repeats: "repeat"
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
        case .repeats(let rule, let interval): Recurrence.describing(rule, every: interval)
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

/// The title as it's typed, with the words Quick Add reads, such as
/// "tomorrow" or "#Health", tinted in place so it's clear which ones the
/// chips stand for.
@available(iOS 26, macOS 26, *)
private struct EntryField: View {
    let title: LocalizedStringKey
    let line: String
    let parts: [QuickEntry.Part]
    let typed: (String) -> Void

    @State private var text = AttributedString()
    @State private var selection = AttributedTextSelection()

    var body: some View {
        TextEditor(text: $text, selection: $selection)
            .scrollDisabled(true)
            .scrollContentBackground(.hidden)
            .fixedSize(horizontal: false, vertical: true)
            .overlay(alignment: .topLeading) {
                if line.isEmpty {
                    Text(title)
                        .foregroundStyle(.tertiary)
                        .padding(.top, Self.inset.top)
                        .padding(.leading, Self.inset.leading)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
            // Lines the words up with the rows below, past the editor's own
            // margin.
            .padding(.horizontal, -Self.inset.leading)
            .accessibilityLabel(Text(title))
            .onAppear { show(line) }
            .onChange(of: text) { passOn() }
            .onChange(of: line) { show(line) }
            .onChange(of: parts) { tint() }
    }

    /// Where the editor starts its text, so the prompt sits where typing
    /// does.
    private static var inset: EdgeInsets {
        #if os(iOS)
            EdgeInsets(top: 8, leading: 5, bottom: 8, trailing: 5)
        #else
            EdgeInsets(top: 0, leading: 5, bottom: 0, trailing: 5)
        #endif
    }

    /// Passes on what was typed. The feature reads a line break; the
    /// field drops it at once, so it stays one line.
    private func passOn() {
        let typed = String(text.characters)
        guard typed != line else { return tint() }
        if typed.contains(where: \.isNewline) { show(QuickAddFeature.oneLine(typed)) }
        self.typed(typed)
    }

    /// Shows `line` when it isn't what's typed, as after an add or a paste.
    private func show(_ line: String) {
        guard String(text.characters) != line else { return }
        text = AttributedString(line)
        selection = AttributedTextSelection(insertionPoint: text.endIndex)
        tint()
    }

    /// Tints the words of `parts`, keeping the cursor where it is. Changes
    /// nothing when they're tinted already, so it doesn't loop.
    private func tint() {
        guard tinted(String(text.characters), parts) != text else { return }
        text.transform(updating: &selection) { text in
            text.foregroundColor = nil
            text.inlinePresentationIntent = nil
            tintParts(&text, parts)
        }
    }
}

/// `line` with the words of `parts` tinted and bold.
private func tinted(_ line: String, _ parts: [QuickEntry.Part]) -> AttributedString {
    var text = AttributedString(line)
    tintParts(&text, parts)
    return text
}

/// Tints and bolds the words of `parts` in `text`.
private func tintParts(_ text: inout AttributedString, _ parts: [QuickEntry.Part]) {
    let characters = text.characters
    for part in parts where part.range.lowerBound >= 0 && part.range.upperBound <= characters.count {
        let start = characters.index(characters.startIndex, offsetBy: part.range.lowerBound)
        let end = characters.index(characters.startIndex, offsetBy: part.range.upperBound)
        text[start..<end].foregroundColor = .accentColor
        text[start..<end].inlinePresentationIntent = .stronglyEmphasized
    }
}
