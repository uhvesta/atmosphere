import KhodrDaemonClient
import SwiftUI

/// Every session waiting on you, across projects: approvals first, then
/// failures, then replies to read. Picking one opens it.
package struct InboxView: View {
    var entries: [InboxEntry]
    var open: (Session.ID) -> Void

    package init(entries: [InboxEntry], open: @escaping (Session.ID) -> Void) {
        self.entries = entries
        self.open = open
    }

    package var body: some View {
        List {
            ForEach(entries) { entry in
                Button {
                    open(entry.session.id)
                } label: {
                    InboxRow(entry: entry)
                }
                .buttonStyle(.plain)
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
