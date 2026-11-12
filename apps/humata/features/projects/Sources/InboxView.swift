import HumataDaemonClient
import SwiftUI

/// Every session waiting on you, across projects: approvals first, then
/// failures, then replies to read. Picking one opens it; a request for
/// permission can be answered from its row.
package struct InboxView: View {
    var entries: [InboxEntry]
    var open: (Session.ID) -> Void
    var answer: (Session.ID, _ request: String, _ approved: Bool) -> Void

    package init(
        entries: [InboxEntry], open: @escaping (Session.ID) -> Void,
        answer: @escaping (Session.ID, _ request: String, _ approved: Bool) -> Void
    ) {
        self.entries = entries
        self.open = open
        self.answer = answer
    }

    package var body: some View {
        List {
            ForEach(entries) { entry in
                HStack(spacing: 12) {
                    Button {
                        open(entry.session.id)
                    } label: {
                        InboxRow(entry: entry)
                    }
                    .buttonStyle(.plain)
                    if entry.reason == .approval, let request = entry.session.pending.first {
                        Button("Deny") { answer(entry.session.id, request.requestID, false) }
                            .help("Deny: \(request.summary)")
                        Button("Allow") { answer(entry.session.id, request.requestID, true) }
                            .buttonStyle(.borderedProminent)
                            .help("Allow: \(request.summary)")
                    }
                }
            }
        }
        .overlay {
            if entries.isEmpty {
                ContentUnavailableView(
                    "Nothing Waiting on You", systemImage: "tray",
                    description: Text("Sessions show here when they ask for approval, fail or finish a turn."))
            }
        }
        .navigationTitle("Inbox")
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
        }
    }

    private var color: Color {
        switch entry.reason {
        case .approval: .orange
        case .failed: .red
        case .reply: .accentColor
        }
    }

    private var reason: String {
        switch entry.reason {
        case .approval: String(localized: "Waiting for approval")
        case .failed: String(localized: "Failed")
        case .reply: String(localized: "Replied")
        }
    }
}
