import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

package struct ApprovalsView: View {
    let store: StoreOf<ApprovalsFeature>

    package init(store: StoreOf<ApprovalsFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                if store.approvals.grants.isEmpty {
                    Text("None: every admin action another program asks for waits for you in the Inbox.")
                        .foregroundStyle(.secondary)
                }
                ForEach(store.approvals.grants) { grant in
                    GrantRow(grant: grant, now: store.now) { store.send(.revokeTapped(grant.id)) }
                }
            } header: {
                Text("Grants in Force")
            } footer: {
                Text(
                    """
                    Admin actions are what the humata command and agents' humata mcp ask to do as you, such as \
                    applying proposals or changing tasks. You grant leave from the Inbox, when you approve one.
                    """
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            Section("Asked Lately") {
                if store.loaded && store.approvals.approvals.isEmpty {
                    Text("No program has asked for an admin action yet.")
                        .foregroundStyle(.secondary)
                }
                ForEach(store.approvals.approvals) { approval in
                    ApprovalHistoryRow(approval: approval)
                }
            }
            if let failure = store.failure {
                Text(failure)
                    .foregroundStyle(.red)
            }
        }
        .formStyle(.grouped)
        .task { await store.send(.task).finish() }
    }
}

private struct GrantRow: View {
    var grant: ApprovalGrant
    var now: Date?
    var revoke: () -> Void

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(grant.scopes.map(\.title).formatted(.list(type: .and)))
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button("Revoke", role: .destructive, action: revoke)
                .help("Have every admin action ask you again from now on")
        }
    }

    private var detail: String {
        let until = grant.expires.formatted(date: .abbreviated, time: .shortened)
        let used =
            grant.used == 1
            ? String(localized: "1 action ran under it") : String(localized: "\(grant.used) actions ran under it")
        return String(localized: "Until \(until) · \(used)")
    }
}

private struct ApprovalHistoryRow: View {
    var approval: AdminApproval

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: icon)
                .foregroundStyle(color)
                .frame(width: 20)
            VStack(alignment: .leading, spacing: 2) {
                Text(approval.summary)
                Text("\(approval.asker) · \(approval.asked.formatted(date: .abbreviated, time: .shortened))")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if let failure = approval.failure {
                    Text(failure)
                        .font(.caption)
                        .foregroundStyle(.red)
                }
            }
            Spacer()
            Text(state)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .combine)
    }

    private var state: String {
        switch approval.state {
        case .pending: String(localized: "Waiting in the Inbox")
        case .denied: String(localized: "Denied")
        case .approved where approval.grantID != nil: String(localized: "Ran under a grant")
        case .approved where approval.failure != nil: String(localized: "Approved; failed")
        case .approved: String(localized: "Approved")
        }
    }

    private var icon: String {
        switch approval.state {
        case .pending: "hourglass"
        case .denied: "xmark.circle"
        case .approved: approval.failure == nil ? "checkmark.circle" : "exclamationmark.triangle"
        }
    }

    private var color: Color {
        switch approval.state {
        case .pending: .orange
        case .denied: .secondary
        case .approved: approval.failure == nil ? .green : .red
        }
    }
}
