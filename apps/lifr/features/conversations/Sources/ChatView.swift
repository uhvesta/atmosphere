import ComposableArchitecture
import LifrAssistantClient
import LifrCore
import SwiftUI

package struct ChatView: View {
    @Bindable package var store: StoreOf<ChatFeature>

    package init(store: StoreOf<ChatFeature>) {
        self.store = store
    }

    package var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 12) {
                if store.availability != .available {
                    Text(store.availability.description)
                        .foregroundStyle(.secondary)
                }
                ForEach(store.conversation.messages) { message in
                    MessageView(message: message, canUndo: store.conversation.changes[message.id] != nil) {
                        store.send(.undoTapped(messageID: message.id))
                    }
                }
                if let replying = store.replying {
                    Text(replying.isEmpty ? "…" : replying)
                        .foregroundStyle(.secondary)
                }
                if let plan = store.conversation.draft {
                    DraftCard(plan: plan, problem: store.problem) {
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
            HStack {
                TextField("Message", text: $store.input, axis: .vertical)
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
            .padding()
            .background(.bar)
        }
        .navigationTitle(store.conversation.title)
        .task { await store.send(.task).finish() }
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}

private struct MessageView: View {
    let message: ChatMessage
    let canUndo: Bool
    let undo: () -> Void

    var body: some View {
        VStack(alignment: message.role == .person ? .trailing : .leading, spacing: 4) {
            Text(message.text)
                .padding(10)
                .background(
                    message.role == .person ? AnyShapeStyle(.tint.opacity(0.2)) : AnyShapeStyle(.fill.tertiary),
                    in: .rect(cornerRadius: 12))
            if canUndo {
                Button("Undo", action: undo)
                    .font(.caption)
            }
        }
        .frame(maxWidth: .infinity, alignment: message.role == .person ? .trailing : .leading)
    }
}

/// The plan to add, as the person will see it in their lists.
private struct DraftCard: View {
    let plan: Plan
    let problem: String?
    let add: () -> Void
    let discard: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Draft").font(.headline)
            ForEach(Array(plan.avenues.enumerated()), id: \.offset) { _, avenue in
                Label("New Avenue \(avenue.name) (\(avenue.key))", systemImage: "square.stack.3d.up")
            }
            ForEach(Array(plan.projects.enumerated()), id: \.offset) { _, project in
                Label("New Project \(project.name) in \(project.avenue)", systemImage: "flag")
                ForEach(Array(project.tasks.enumerated()), id: \.offset) { _, task in
                    TaskLine(task: task).padding(.leading)
                }
            }
            ForEach(Array(plan.tasks.enumerated()), id: \.offset) { _, task in
                TaskLine(task: task)
                ForEach(Array(task.subtasks.enumerated()), id: \.offset) { _, subtask in
                    TaskLine(task: subtask).padding(.leading)
                }
            }
            if let problem {
                Text(problem).font(.caption).foregroundStyle(.red)
            }
            HStack {
                Button("Discard", role: .destructive, action: discard)
                Spacer()
                Button("Add", action: add).buttonStyle(.borderedProminent)
            }
        }
        .padding()
        .background(.fill.quaternary, in: .rect(cornerRadius: 12))
    }
}

private struct TaskLine: View {
    let task: Plan.TaskDraft

    var body: some View {
        VStack(alignment: .leading) {
            Label(task.title, systemImage: "circle")
            let details = [task.place, task.parent.map { "under \($0)" }, task.due.map { "due \($0)" }]
                .compactMap(\.self)
            if !details.isEmpty {
                Text(details.joined(separator: " · ")).font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}
