import ChatKit
import ChatUI
import ComposableArchitecture
import InferenceKit
import KhodrDaemonClient
import SwiftUI

/// A session's chat: the person's messages, the agent's thinking, replies,
/// the commands it runs with their output and the permissions it asks for,
/// with the field to send, steer or queue.
package struct SessionView: View {
    @Bindable var store: StoreOf<SessionFeature>
    @State private var selecting: TextToSelect?

    package init(store: StoreOf<SessionFeature>) {
        self.store = store
    }

    package var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            chat
            if store.disconnected {
                ChatNotice(
                    "Lost the connection to Khodr's daemon.", systemImage: "bolt.horizontal.circle",
                    action: "Reconnect"
                ) { store.send(.task) }
            }
            composer
                .padding([.horizontal, .bottom])
        }
        .task { await store.send(.task).finish() }
        .alert($store.scope(state: \.alert, action: \.alert))
        .sheet(item: $selecting) { SelectTextSheet(text: $0.text) }
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(store.session.name.isEmpty ? "Session" : store.session.name)
                    .font(.headline)
                Text(engine)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            if let usage = store.usage {
                Text(Self.describe(usage))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .help("What this session has used")
            }
            Picker("Approvals", selection: approvalMode) {
                ForEach(ApprovalMode.allCases, id: \.self) { mode in
                    Text(Self.name(mode)).tag(mode)
                }
            }
            .pickerStyle(.menu)
            .fixedSize()
            .help("What the agent may do without asking")
        }
        .padding(.horizontal)
        .padding(.vertical, 10)
    }

    private var approvalMode: Binding<ApprovalMode> {
        Binding(get: { store.session.approvalMode }, set: { store.send(.approvalModePicked($0)) })
    }

    private var engine: String {
        let backend =
            switch store.session.backend {
            case .copilot: "Copilot"
            case .local: "Local model"
            case .scripted: "Scripted"
            }
        let model = store.session.model.map { " · \($0)" } ?? ""
        let host = store.session.host.map { " on \($0)" } ?? ""
        return backend + model + host
    }

    private var chat: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 14) {
                ForEach(store.items) { item in
                    row(item)
                }
                if store.isWorking, !store.isWaiting {
                    ReplyInProgress(
                        text: store.streaming, thinking: nil, showsThinking: false, toggleThinking: {})
                }
            }
            .padding()
        }
        .defaultScrollAnchor(.bottom)
    }

    @ViewBuilder
    private func row(_ item: ChatItem) -> some View {
        switch item {
        case .person(_, let text):
            MessageView(role: .person, text: text, select: { selecting = TextToSelect(text: $0) })
        case .reply(_, let text):
            MessageView(role: .assistant, text: text, select: { selecting = TextToSelect(text: $0) })
        case .thinking(let id, let text):
            ThinkingDisclosure(
                thinking: ModelThinking(text: text), isExpanded: store.expandedThinking.contains(id),
                toggle: { store.send(.thinkingToggled(id: id)) }, select: { selecting = TextToSelect(text: $0) },
                answering: true)
        case .tool(let call):
            ToolCallView(call: call) { request, approved in
                store.send(.answerPermission(requestID: request, approved: approved))
            }
        case .permission(_, let permission):
            PermissionCard(permission: permission) { request, approved in
                store.send(.answerPermission(requestID: request, approved: approved))
            }
        case .stopped:
            Label("Stopped", systemImage: "stop.circle")
                .font(.footnote)
                .foregroundStyle(.secondary)
        case .failure(_, let message):
            Label(message, systemImage: "exclamationmark.triangle")
                .font(.footnote)
                .foregroundStyle(.red)
        }
    }

    private var composer: some View {
        ChatComposer(
            text: $store.draft, prompt: Text(store.isWorking ? "Queue a message, or steer" : "Message the agent"),
            model: nil,
            send: SendButton(
                canSend: !store.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                canStop: store.isWorking,
                send: { store.send(.sendTapped(.enqueue)) },
                stop: { store.send(.abortTapped) })
        ) {
            store.send(.sendTapped(.enqueue))
        } top: {
            if store.isWorking, !store.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                HStack {
                    Text("Return queues it for after this turn.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("Steer Now") { store.send(.sendTapped(.steer)) }
                        .font(.caption.weight(.semibold))
                        .help("Send it to the running turn")
                }
                .padding(.horizontal, 4)
            }
        }
    }

    static func name(_ mode: ApprovalMode) -> String {
        switch mode {
        case .askEveryTime: "Ask Every Time"
        case .allowEdits: "Allow Edits"
        case .allowEverything: "Allow Everything"
        }
    }

    static func describe(_ usage: Usage) -> String {
        let tokens =
            "\(usage.inputTokens.formatted(.number.notation(.compactName))) in · "
            + "\(usage.outputTokens.formatted(.number.notation(.compactName))) out"
        guard usage.premiumRequests > 0 else { return "\(usage.model) · \(tokens)" }
        return "\(usage.model) · \(tokens) · \(usage.premiumRequests.formatted()) premium"
    }
}

/// A tool the agent called: what it ran or wrote, the permission it asked
/// for, and its output, folded away until opened.
struct ToolCallView: View {
    let call: ToolCall
    let answer: (String, Bool) -> Void
    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                expanded.toggle()
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: icon)
                        .foregroundStyle(tint)
                    Text(call.subject ?? call.tool)
                        .font(.callout.monospaced())
                        .lineLimit(1)
                        .truncationMode(.middle)
                    if call.output == nil, call.permission?.answer != nil || call.permission == nil {
                        ProgressView().controlSize(.mini)
                    }
                    Spacer()
                    if call.output != nil {
                        Image(systemName: "chevron.right")
                            .font(.caption.weight(.semibold))
                            .rotationEffect(.degrees(expanded ? 90 : 0))
                            .foregroundStyle(.secondary)
                    }
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .disabled(call.output == nil)
            if let permission = call.permission {
                PermissionCard(permission: permission, answer: answer)
            }
            if expanded, let output = call.output {
                Text(output)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
                    .background(.fill.quinary, in: .rect(cornerRadius: 8))
            }
        }
        .padding(10)
        .background(.fill.quaternary.opacity(0.5), in: .rect(cornerRadius: 12))
    }

    private var icon: String {
        switch call.succeeded {
        case true?: "checkmark.circle.fill"
        case false?: "xmark.circle.fill"
        case nil: call.tool == "shell" ? "terminal" : "doc.text"
        }
    }

    private var tint: Color {
        switch call.succeeded {
        case true?: .green
        case false?: .red
        case nil: .secondary
        }
    }
}

/// A request for permission: Approve and Deny while the agent waits, then
/// how it was answered.
struct PermissionCard: View {
    let permission: Permission
    let answer: (String, Bool) -> Void

    var body: some View {
        switch permission.answer {
        case nil:
            HStack(spacing: 8) {
                Label(permission.summary, systemImage: "hand.raised")
                    .font(.callout)
                    .lineLimit(2)
                Spacer()
                Button("Deny") { answer(permission.requestID, false) }
                Button("Approve") { answer(permission.requestID, true) }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
            }
            .padding(8)
            .background(.orange.opacity(0.12), in: .rect(cornerRadius: 8))
        case .approved?:
            answered("Approved", systemImage: "checkmark")
        case .denied?:
            answered("Denied", systemImage: "xmark")
        case .allowedByMode?:
            answered("Allowed by the approval mode", systemImage: "checkmark.shield")
        }
    }

    private func answered(_ text: LocalizedStringKey, systemImage: String) -> some View {
        Label(text, systemImage: systemImage)
            .font(.caption)
            .foregroundStyle(.secondary)
    }
}

/// Text opened to select part of.
private struct TextToSelect: Identifiable {
    let id = UUID()
    let text: String
}
