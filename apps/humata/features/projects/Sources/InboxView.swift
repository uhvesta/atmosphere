import HumataDaemonClient
import SwiftUI

/// Every session waiting on you, across projects: first those blocked on you
/// (approvals, agents' questions and moves, failures), then replies to look
/// over. Picking one opens it; a request for permission, a question or a move
/// can be answered from its row, and a reply marked as read.
package struct InboxView: View {
    var entries: [InboxEntry]
    var open: (Session.ID) -> Void
    var openChanges: (Session.ID) -> Void
    var answer: (Session.ID, _ request: String, _ approved: Bool) -> Void
    var answerRequest: (AgentRequest.ID, RequestAnswer) -> Void
    var drafts: [AgentRequest.ID: String]
    var draftChanged: (AgentRequest.ID, String) -> Void
    var markRead: (Session.ID) -> Void

    package init(
        entries: [InboxEntry], drafts: [AgentRequest.ID: String], open: @escaping (Session.ID) -> Void,
        openChanges: @escaping (Session.ID) -> Void,
        answer: @escaping (Session.ID, _ request: String, _ approved: Bool) -> Void,
        answerRequest: @escaping (AgentRequest.ID, RequestAnswer) -> Void,
        draftChanged: @escaping (AgentRequest.ID, String) -> Void,
        markRead: @escaping (Session.ID) -> Void
    ) {
        self.entries = entries
        self.drafts = drafts
        self.open = open
        self.openChanges = openChanges
        self.answer = answer
        self.answerRequest = answerRequest
        self.draftChanged = draftChanged
        self.markRead = markRead
    }

    package var body: some View {
        List {
            let blocked = entries.filter(\.reason.blocks)
            let toLookOver = entries.filter { !$0.reason.blocks }
            if !blocked.isEmpty {
                Section("Blocked on You") {
                    ForEach(blocked, content: row)
                }
            }
            if !toLookOver.isEmpty {
                Section("To Look Over") {
                    ForEach(toLookOver, content: row)
                }
            }
        }
        .overlay {
            if entries.isEmpty {
                ContentUnavailableView(
                    "Nothing Waiting on You", systemImage: "tray",
                    description: Text(
                        "Sessions show here when they ask you something, ask for approval, fail or finish a turn."))
            }
        }
        .navigationTitle("Inbox")
    }

    @ViewBuilder
    private func row(_ entry: InboxEntry) -> some View {
        if let request = entry.request {
            RequestRow(
                entry: entry, request: request,
                reply: Binding(get: { drafts[request.id] ?? "" }, set: { draftChanged(request.id, $0) }),
                open: { open(entry.session.id) }, openChanges: { openChanges(entry.session.id) },
                answer: { answerRequest(request.id, $0) })
        } else {
            sessionRow(entry)
        }
    }

    private func sessionRow(_ entry: InboxEntry) -> some View {
        HStack(spacing: 12) {
            Button {
                open(entry.session.id)
            } label: {
                InboxRow(entry: entry)
            }
            .buttonStyle(.plain)
            if entry.reason == .approval, entry.session.hostOffline {
                // An answer couldn't reach the agent; it waits for the host.
                Label("Host Offline", systemImage: "wifi.slash")
                    .foregroundStyle(.secondary)
                    .help("Its host isn't answering; you can answer once it's back")
            } else if entry.reason == .approval, let request = entry.session.pending.first {
                Button("Deny") { answer(entry.session.id, request.requestID, false) }
                    .help("Deny: \(request.summary)")
                Button("Allow") { answer(entry.session.id, request.requestID, true) }
                    .buttonStyle(.borderedProminent)
                    .help("Allow: \(request.summary)")
            } else if entry.reason == .reply {
                Button("Mark as Read") { markRead(entry.session.id) }
                    .help("Take it out of the inbox without opening it")
            }
        }
    }
}

/// An agent's question, answered with a reply, or its finished work, which
/// the person looks over and moves, or sends feedback on.
private struct RequestRow: View {
    var entry: InboxEntry
    var request: AgentRequest
    @Binding var reply: String
    var open: () -> Void
    var openChanges: () -> Void
    var answer: (RequestAnswer) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button(action: open) {
                InboxRow(entry: entry)
            }
            .buttonStyle(.plain)
            Text(LocalizedStringKey(request.text))
                .textSelection(.enabled)
                .padding(.leading, 30)
            HStack {
                TextField(isQuestion ? "Reply" : "Feedback to carry on from", text: $reply)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(sendReply)
                Button(isQuestion ? "Reply" : "Send Feedback", action: sendReply)
                    .disabled(trimmed.isEmpty)
                if case .move(_, let status) = request.kind {
                    Button("Open Changes", action: openChanges)
                    Menu("Move To") {
                        ForEach(ItemStatus.allCases.filter { $0 != status }, id: \.self) { other in
                            Button(other.title) { answer(.move(other)) }
                        }
                    }
                    .fixedSize()
                    Button("Move to \(status.title)") { answer(.approve) }
                        .buttonStyle(.borderedProminent)
                } else {
                    Button("Dismiss") { answer(.dismiss) }
                }
            }
            .padding(.leading, 30)
        }
        .padding(.vertical, 4)
    }

    private var isQuestion: Bool { request.kind == .question }

    private var trimmed: String { reply.trimmingCharacters(in: .whitespacesAndNewlines) }

    private func sendReply() {
        // The draft stays until the answer reaches the agent.
        guard !trimmed.isEmpty else { return }
        answer(.reply(trimmed))
    }
}

private struct InboxRow: View {
    var entry: InboxEntry

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: icon)
                .foregroundStyle(color)
                .frame(width: 20)
            VStack(alignment: .leading, spacing: 2) {
                Text(entry.session.title)
                if entry.reason == .approval, let request = entry.session.pending.first {
                    Text(request.summary)
                        .font(.callout.monospaced())
                        .lineLimit(2)
                        .truncationMode(.middle)
                    if entry.session.pending.count > 1 {
                        Text("and \(entry.session.pending.count - 1) more")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                if entry.reason == .failed || entry.reason == .reply, let message = entry.session.lastMessage {
                    Text(message)
                        .font(.callout)
                        .foregroundStyle(entry.reason == .failed ? Color.red : Color.secondary)
                        .lineLimit(3)
                }
                Text(place)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Text(reason)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }

    private var place: String {
        [entry.projectName, entry.itemTitle].compactMap(\.self).joined(separator: " › ")
    }

    private var icon: String {
        switch entry.reason {
        case .approval: "hand.raised.fill"
        case .failed: "exclamationmark.triangle.fill"
        case .reply: "text.bubble"
        case .question: "questionmark.bubble.fill"
        case .move: "checkmark.seal.fill"
        }
    }

    private var color: Color {
        switch entry.reason {
        case .approval: .orange
        case .failed: .red
        case .reply: .accentColor
        case .question: .purple
        case .move: .green
        }
    }

    private var reason: String {
        switch entry.reason {
        case .approval: String(localized: "Waiting for approval")
        case .failed: String(localized: "Failed")
        case .reply: String(localized: "Replied")
        case .question: String(localized: "Asks you")
        case .move: String(localized: "Asks to move it to \(entry.request?.kind.statusName ?? "")")
        }
    }
}
