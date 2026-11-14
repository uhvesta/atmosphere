import ChatUI
import ComposableArchitecture
import LifrAssistantClient
import LifrModelsClient
import LifrUI
import SwiftUI

package struct SkillsView: View {
    @Bindable package var store: StoreOf<SkillsFeature>

    package init(store: StoreOf<SkillsFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            if let loadError = store.loadError {
                Section {
                    Text(loadError).foregroundStyle(.red)
                    Button("Try Again") { store.send(.task) }
                } header: {
                    Text("Couldn't load your skills")
                }
            }
            Section {
                ForEach(store.skills.filter { !$0.isUser }) { skill in
                    toggle(skill)
                        .contextMenu { autoPicks(skill) }
                }
            } header: {
                Text("Built in")
            } footer: {
                Text(
                    "Pick a skill for a message to tell the assistant what to do and which tools it may use. Skills "
                        + "marked Automatic are picked for you when Settings › Assistant › Pick a skill "
                        + "automatically is on; touch and hold one to change that.")
            }
            Section {
                ForEach(store.skills.filter(\.isUser)) { skill in
                    toggle(skill)
                        .contentShape(.rect)
                        .onTapGesture { store.send(.editTapped(skill.id)) }
                        .swipeActions(allowsFullSwipe: false) {
                            Button("Delete", role: .destructive) { store.send(.deleteTapped([skill.id])) }
                        }
                        .contextMenu {
                            Button("Edit") { store.send(.editTapped(skill.id)) }
                            autoPicks(skill)
                            Button("Delete", role: .destructive) { store.send(.deleteTapped([skill.id])) }
                        }
                }
                Button("New Skill") { store.send(.newTapped) }
            } header: {
                Text("Yours")
            } footer: {
                Text("Your skills sync through iCloud, encrypted end to end.")
            }
        }
        .navigationTitle("Skills")
        .task { await store.send(.task).finish() }
        .sheet(item: $store.scope(state: \.editor, action: \.editor)) { store in
            SkillEditorView(store: store)
                .sheetLayout(height: 520)
        }
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    private func toggle(_ skill: Skill) -> some View {
        SkillToggle(skill) { store.send(.toggled(skill.id, $0)) }
    }

    private func autoPicks(_ skill: Skill) -> some View {
        Button(skill.autoPicks ? "Don't Use Automatically" : "Use Automatically") {
            store.send(.autoPicksToggled(skill.id, !skill.autoPicks))
        }
    }
}

package struct SkillEditorView: View {
    @Bindable package var store: StoreOf<SkillEditor>
    @Environment(\.dismiss) private var dismiss

    package init(store: StoreOf<SkillEditor>) {
        self.store = store
    }

    package var body: some View {
        NavigationStack {
            Form {
                assist
                Section {
                    if let skill = store.editing {
                        LabeledContent("Name", value: skill.id)
                    } else {
                        TextField("Name", text: $store.name, prompt: Text("Weekly review"))
                    }
                    TextField("Summary", text: $store.summary, prompt: Text("What it's for, in a line"))
                } footer: {
                    if let problem = store.problem {
                        Text(problem).foregroundStyle(.red)
                    }
                }
                Section {
                    TextField(
                        "Instructions", text: $store.instructions,
                        prompt: Text("Go through what's overdue and suggest new dates."), axis: .vertical
                    )
                    .lineLimit(4...12)
                } header: {
                    Text("Instructions")
                } footer: {
                    Text("What the assistant does for a message that uses this skill.")
                }
                Section {
                    ForEach(AssistantTool.allCases, id: \.self) { tool in
                        Toggle(
                            tool.title,
                            isOn: Binding(
                                get: { store.tools.contains(tool) }, set: { store.send(.toolToggled(tool, $0)) }))
                    }
                    Toggle("Draft plans", isOn: $store.draftsPlans)
                } header: {
                    Text("The assistant may")
                } footer: {
                    Text("Changes save at once, and each can be undone from the conversation or Activity.")
                }
                Section {
                    Toggle("Use automatically", isOn: $store.autoPicks)
                    if store.autoPicks {
                        TextField(
                            "Use when", text: $store.useWhen,
                            prompt: Text("I ask to look back over my week"), axis: .vertical
                        )
                        .lineLimit(1...4)
                    }
                } footer: {
                    Text(
                        "When Pick a skill automatically is on in Settings, Lifr picks this skill for a message that "
                            + "fits Use When, or its summary if that's empty.")
                }
                Section {
                    TextField(
                        "Goal", text: $store.planGoal, prompt: Text("Plan next week's errands"), axis: .vertical
                    )
                    .lineLimit(1...4)
                    if store.keepsScope {
                        Button("Forget the Tasks Listed", systemImage: "xmark.circle") {
                            store.send(.binding(.set(\.keepsScope, false)))
                        }
                    }
                } header: {
                    Text("Planning with Another Assistant")
                } footer: {
                    if store.keepsScope {
                        Text("A plan prompt made with this skill starts from this goal and the tasks you kept.")
                    } else {
                        Text("A plan prompt made with this skill starts from this goal.")
                    }
                }
            }
            .disabled(store.isDrafting)
            .navigationTitle(store.editing == nil ? Text("New Skill") : Text("Edit Skill"))
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { store.send(.saveTapped) }
                        .disabled(!store.canSave)
                }
            }
        }
    }

    /// The assistant's help, off until the person asks for it. It fills in
    /// the fields; nothing is saved until they tap Save.
    @ViewBuilder private var assist: some View {
        Section {
            if store.assists {
                TextField(
                    "Describe the skill", text: $store.request,
                    prompt: Text("Every Friday, show what I finished and what's overdue"), axis: .vertical
                )
                .lineLimit(2...5)
                ModelPicker(
                    selection: $store.model, models: store.models,
                    standard: store.models.first { $0.id == store.standard }?.name)
                Button {
                    store.send(.draftTapped)
                } label: {
                    if store.isDrafting {
                        HStack {
                            ProgressView()
                            Text("Drafting…")
                        }
                    } else {
                        Text(store.editing == nil ? "Draft" : "Redraft")
                    }
                }
                .disabled(!store.canDraft)
            } else {
                Button {
                    store.send(.assistTapped)
                } label: {
                    Label("Draft with Apple Intelligence", systemImage: "sparkles")
                }
            }
        } footer: {
            if let problem = store.draftProblem {
                Text(problem).foregroundStyle(.red)
            } else if store.assists {
                Text("The draft fills in the fields below on this device. Check it before you save.")
            }
        }
    }
}
