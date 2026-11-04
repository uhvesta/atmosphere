import ComposableArchitecture
import LifrAssistantClient
import LifrCore
import LifrSkills
import SwiftUI

package struct ChatView: View {
    @Bindable package var store: StoreOf<ChatFeature>

    package init(store: StoreOf<ChatFeature>) {
        self.store = store
    }

    package var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if store.availability != .available {
                    Text(store.availability.description)
                        .foregroundStyle(.secondary)
                }
                ForEach(store.conversation.messages) { message in
                    MessageView(
                        message: message, skill: store.state.skillTitle(of: message),
                        canUndo: store.conversation.changes[message.id] != nil
                    ) {
                        store.send(.undoTapped(messageID: message.id))
                    }
                }
                if let replying = store.replying {
                    Text(replying.isEmpty ? "…" : replying)
                        .foregroundStyle(.secondary)
                }
                if let plan = Binding($store.conversation.draft) {
                    DraftCard(plan: plan, problem: store.problem, canAdd: store.canAdd) {
                        store.send(.addTapped)
                    } discard: {
                        store.send(.discardDraftTapped)
                    }
                }
            }
            .padding()
        }
        .defaultScrollAnchor(.bottom)
        .safeAreaInset(edge: .bottom) {
            composer
                .padding()
                .background(.bar)
        }
        .sheet(isPresented: $store.findsSkill) {
            SkillFinder(store: store)
        }
        .sheet(item: $store.scope(state: \.manageSkills, action: \.manageSkills)) { store in
            NavigationStack {
                SkillsView(store: store)
                    .toolbar {
                        ToolbarItem(placement: .confirmationAction) {
                            DismissButton()
                        }
                    }
            }
        }
        .navigationTitle(store.conversation.title)
        .task { await store.send(.task).finish() }
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    /// The message field, with the skill for the next message: a pill once
    /// one is picked, a button to find one before.
    private var composer: some View {
        HStack {
            if let skill = store.picked {
                SkillPill(title: skill.title) {
                    store.send(.findSkillTapped)
                } remove: {
                    store.send(.clearSkillTapped)
                }
            } else {
                Button {
                    store.send(.findSkillTapped)
                } label: {
                    Label("Use a Skill", systemImage: "sparkles")
                        .labelStyle(.iconOnly)
                        .font(.title2)
                }
            }
            TextField(
                "Message", text: $store.input, prompt: store.picked.map { _ in Text("Add details, or just send") },
                axis: .vertical
            )
            .textFieldStyle(.roundedBorder)
            .onSubmit { store.send(.sendTapped) }
            Button {
                store.send(.sendTapped)
            } label: {
                Label("Send", systemImage: "arrow.up.circle.fill")
                    .labelStyle(.iconOnly)
                    .font(.title2)
            }
            .disabled(!store.canSend)
        }
    }
}

private struct DismissButton: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Button("Done") { dismiss() }
    }
}

/// The skill picked for the next message: tap it to pick another, or
/// remove it.
private struct SkillPill: View {
    let title: String
    let change: () -> Void
    let remove: () -> Void

    var body: some View {
        HStack(spacing: 4) {
            Button(action: change) {
                Label(title, systemImage: "sparkles")
                    .font(.callout.weight(.medium))
            }
            .buttonStyle(.plain)
            .accessibilityHint(Text("Picks another skill"))
            Button(action: remove) {
                Label("Remove \(title)", systemImage: "xmark.circle.fill")
                    .labelStyle(.iconOnly)
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(.tint.opacity(0.2), in: .capsule)
    }
}

/// Finds a skill for the next message by its name or what it does.
private struct SkillFinder: View {
    @Bindable var store: StoreOf<ChatFeature>

    var body: some View {
        NavigationStack {
            List {
                ForEach(store.skillChoices) { skill in
                    Button {
                        store.send(.pickSkillTapped(skill.id))
                    } label: {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(skill.title)
                            Text(skill.summary)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                Section {
                    Button("Manage Skills…") { store.send(.manageSkillsTapped) }
                }
            }
            .overlay {
                if store.skillChoices.isEmpty, !store.skillQuery.isEmpty {
                    ContentUnavailableView.search(text: store.skillQuery)
                }
            }
            .searchable(text: $store.skillQuery, prompt: Text("Find a skill"))
            .navigationTitle("Skills")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { store.findsSkill = false }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }
}

private struct MessageView: View {
    let message: ChatMessage
    let skill: String?
    let canUndo: Bool
    let undo: () -> Void

    var body: some View {
        VStack(alignment: message.role == .person ? .trailing : .leading, spacing: 4) {
            if let skill {
                Label(skill, systemImage: "sparkles")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.tint)
            }
            if !message.text.isEmpty {
                Text(message.text)
                    .padding(10)
                    .background(
                        message.role == .person ? AnyShapeStyle(.tint.opacity(0.2)) : AnyShapeStyle(.fill.tertiary),
                        in: .rect(cornerRadius: 12))
            }
            if canUndo {
                Button("Undo", action: undo)
                    .font(.caption)
            }
        }
        .frame(maxWidth: .infinity, alignment: message.role == .person ? .trailing : .leading)
    }
}
