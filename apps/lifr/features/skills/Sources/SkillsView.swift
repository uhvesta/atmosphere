import ComposableArchitecture
import LifrAssistantClient
import SwiftUI

package struct SkillsView: View {
    @Bindable package var store: StoreOf<SkillsFeature>

    package init(store: StoreOf<SkillsFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                ForEach(store.skills.filter { !$0.isUser }) { skill in
                    toggle(skill)
                }
            } header: {
                Text("Built in")
            } footer: {
                Text("Pick a skill for a message to tell the assistant what to do and which tools it may use.")
            }
            Section {
                ForEach(store.skills.filter(\.isUser)) { skill in
                    toggle(skill)
                        .contentShape(.rect)
                        .onTapGesture { store.send(.editTapped(skill.id)) }
                        .swipeActions {
                            Button("Delete", role: .destructive) { store.send(.deleteTapped([skill.id])) }
                        }
                        .contextMenu {
                            Button("Edit") { store.send(.editTapped(skill.id)) }
                            Button("Delete", role: .destructive) { store.send(.deleteTapped([skill.id])) }
                        }
                }
                Button("New Skill") { store.send(.newTapped) }
            } header: {
                Text("Yours")
            } footer: {
                Text("Your skills sync with conversations when Sync conversations is on.")
            }
        }
        .navigationTitle("Skills")
        .task { await store.send(.task).finish() }
        .sheet(item: $store.scope(state: \.editor, action: \.editor)) { store in
            SkillEditorView(store: store)
        }
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    private func toggle(_ skill: Skill) -> some View {
        Toggle(
            isOn: Binding(get: { skill.isOn }, set: { store.send(.toggled(skill.id, $0)) })
        ) {
            VStack(alignment: .leading, spacing: 2) {
                Text(skill.title)
                Text(skill.summary)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
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
