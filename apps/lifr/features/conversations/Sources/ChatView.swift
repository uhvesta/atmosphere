import ComposableArchitecture
import LifrAssistantClient
import LifrCore
import LifrSkills
import LifrUI
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
                if let draft = store.conversation.draft {
                    DraftCard(
                        plan: $store.conversation.draft.or(draft), workspace: store.workspace, today: store.today,
                        problem: store.problem, canAdd: store.canAdd
                    ) {
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
        #if os(iOS)
            .sheet(isPresented: $store.findsSkill) {
                SkillFinder(store: store)
            }
        #endif
        .sheet(item: $store.scope(state: \.manageSkills, action: \.manageSkills)) { store in
            NavigationStack {
                SkillsView(store: store)
                    .toolbar {
                        ToolbarItem(placement: .confirmationAction) {
                            DismissButton()
                        }
                    }
            }
            .sheetLayout(height: 520)
        }
        .navigationTitle(store.conversation.title)
        .toolbarTitleMenu {
            Button("Rename", systemImage: "pencil") { store.send(.renameTapped) }
        }
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
        #if os(macOS)
            // On the Mac the finder hangs off the composer, where the
            // button is, instead of covering the chat.
            .popover(isPresented: $store.findsSkill, arrowEdge: .top) {
                SkillFinder(store: store)
            }
        #endif
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
        #if os(macOS)
            VStack(spacing: 0) {
                TextField("Find a skill", text: $store.skillQuery)
                    .textFieldStyle(.roundedBorder)
                    .padding(10)
                list
                Divider()
                HStack {
                    Button("Manage Skills…") { store.send(.manageSkillsTapped) }
                    Spacer()
                    Button("Cancel") { store.findsSkill = false }
                        .keyboardShortcut(.cancelAction)
                }
                .padding(10)
            }
            .frame(width: 340, height: 420)
        #else
            NavigationStack {
                // The search field and Manage Skills sit above the list:
                // at the bottom they'd float over its last rows.
                list
                    .searchable(
                        text: $store.skillQuery, placement: .navigationBarDrawer(displayMode: .always),
                        prompt: Text("Find a skill")
                    )
                    .navigationTitle("Skills")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") { store.findsSkill = false }
                        }
                        ToolbarItem(placement: .primaryAction) {
                            Button("Manage Skills…") { store.send(.manageSkillsTapped) }
                        }
                    }
            }
            .presentationDetents([.medium, .large])
        #endif
    }

    private var list: some View {
        List(store.skillChoices) { skill in
            Button {
                store.send(.pickSkillTapped(skill.id))
            } label: {
                VStack(alignment: .leading, spacing: 2) {
                    Text(skill.title)
                    Text(skill.summary)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
        }
        .overlay {
            if store.skillChoices.isEmpty {
                if store.skillQuery.isEmpty {
                    ContentUnavailableView(
                        "No skills on", systemImage: "sparkles",
                        description: Text("Turn skills on in Manage Skills."))
                } else {
                    ContentUnavailableView.search(text: store.skillQuery)
                }
            }
        }
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

extension Binding {
    /// The value, or `fallback` once it's gone, with writes dropped then,
    /// while SwiftUI catches up. SwiftUI's own `Binding(_:)` over an
    /// optional force-unwraps it, which traps if the view reads it once more
    /// after it's set to nil, as Discard does.
    fileprivate func or<Wrapped>(_ fallback: Wrapped) -> Binding<Wrapped> where Value == Wrapped? {
        Binding<Wrapped>(
            get: { wrappedValue ?? fallback },
            set: { if wrappedValue != nil { wrappedValue = $0 } })
    }
}
