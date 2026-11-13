import HumataDaemonClient
import SwiftUI

/// How long an approval can let the actions after it run without asking.
let grantDurations: [(title: String, duration: Duration)] = [
    (String(localized: "15 Minutes"), .seconds(15 * 60)),
    (String(localized: "1 Hour"), .seconds(60 * 60)),
    (String(localized: "8 Hours"), .seconds(8 * 60 * 60)),
]

/// The admin actions one program asked for, as `humata mcp` under an agent:
/// each approved or denied from its row, or all of them at once.
struct AdminApprovalsSection: View {
    var asker: String
    var approvals: [AdminApproval]
    var now: Date?
    /// The approval answered by ⇧⌘A and ⇧⌘D, if it's one of these.
    var shortcuts: AdminApproval.ID?
    var decide: ([AdminApproval.ID], _ approve: Bool, ApprovalGrantAsked?) -> Void

    var body: some View {
        Section {
            ForEach(approvals) { approval in
                AdminApprovalRow(approval: approval, now: now, shortcuts: approval.id == shortcuts) { approve, grant in
                    decide([approval.id], approve, grant)
                }
            }
        } header: {
            HStack {
                Text("\(asker) Asks to Act as You")
                Spacer()
                if approvals.count > 1 {
                    DecideButtons(
                        deny: String(localized: "Deny All"), approve: String(localized: "Approve All"),
                        scopes: scopes
                    ) { approve, grant in
                        decide(approvals.map(\.id), approve, grant)
                    }
                    .controlSize(.small)
                }
            }
        }
    }

    /// The scopes its actions need, for a grant that covers them all.
    private var scopes: [ApprovalScope] {
        ApprovalScope.allCases.filter { scope in approvals.contains { $0.scope == scope } }
    }
}

/// Marks the row ⇧⌘A and ⇧⌘D answer, so the person sees which one a press
/// goes to before pressing.
struct ShortcutHint: View {
    /// Whether the row can be denied with ⇧⌘D as well.
    var deny = true

    var body: some View {
        Text(verbatim: deny ? "⇧⌘D  ⇧⌘A" : "⇧⌘A")
            .font(.caption)
            .foregroundStyle(.secondary)
            .accessibilityHidden(true)
    }
}

/// Deny, Approve, and a menu to approve and let actions of `scopes` run
/// without asking for a while.
private struct DecideButtons: View {
    var deny: String
    var approve: String
    var scopes: [ApprovalScope]
    /// Whether ⇧⌘A and ⇧⌘D answer it.
    var shortcuts = false
    var decide: (_ approve: Bool, ApprovalGrantAsked?) -> Void

    var body: some View {
        HStack {
            if shortcuts {
                ShortcutHint()
                Button(deny) { decide(false, nil) }
                    .keyboardShortcut("d", modifiers: [.command, .shift])
                    .help("\(deny) (⇧⌘D)")
                Button(approve) { decide(true, nil) }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut("a", modifiers: [.command, .shift])
                    .help("\(approve) (⇧⌘A)")
            } else {
                Button(deny) { decide(false, nil) }
                Button(approve) { decide(true, nil) }
                    .buttonStyle(.borderedProminent)
            }
            Menu {
                Section("Approve, and Allow \(scopeNames) Without Asking For") {
                    ForEach(grantDurations, id: \.title) { choice in
                        Button(choice.title) {
                            decide(true, ApprovalGrantAsked(scopes: scopes, duration: choice.duration))
                        }
                    }
                }
            } label: {
                Image(systemName: "chevron.down")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("Approve, and stop asking for a while; revoke it in Settings › Approvals")
        }
    }

    private var scopeNames: String {
        scopes.map(\.title).formatted(.list(type: .and))
    }
}

private struct AdminApprovalRow: View {
    var approval: AdminApproval
    var now: Date?
    var shortcuts: Bool
    var decide: (_ approve: Bool, ApprovalGrantAsked?) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: approval.scope == .destructive ? "exclamationmark.shield.fill" : "lock.shield")
                    .foregroundStyle(approval.scope == .destructive ? Color.red : Color.orange)
                    .frame(width: 20)
                VStack(alignment: .leading, spacing: 2) {
                    Text(approval.summary)
                        .textSelection(.enabled)
                    Text(approval.scope.title)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if let waited {
                    Text(waited)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            if !approval.before.isEmpty || !approval.after.isEmpty {
                TextChange(before: approval.before, after: approval.after)
                    .padding(.leading, 30)
            }
            HStack {
                Spacer()
                DecideButtons(
                    deny: String(localized: "Deny"), approve: String(localized: "Approve"), scopes: [approval.scope],
                    shortcuts: shortcuts, decide: decide)
            }
        }
        .padding(.vertical, 4)
    }

    private var waited: String? {
        guard let now else { return nil }
        let seconds = max(0, now.timeIntervalSince(approval.asked))
        guard seconds >= 60 else { return String(localized: "just now") }
        let formatter = DateComponentsFormatter()
        formatter.unitsStyle = .abbreviated
        formatter.maximumUnitCount = 1
        formatter.allowedUnits = [.day, .hour, .minute]
        return formatter.string(from: seconds).map { String(localized: "waited \($0)") }
    }
}

/// The lines a title or spec loses and gains.
private struct TextChange: View {
    var before: String
    var after: String
    /// Past this the rest scrolls, so one long spec doesn't push the others away.
    private static let height: CGFloat = 12 * 17

    var body: some View {
        let lines = Self.lines(before, after)
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                    Text("\(line.removed ? "-" : "+") \(line.text)")
                        .font(.caption.monospaced())
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 6)
                        .background(line.removed ? Color.red.opacity(0.15) : Color.green.opacity(0.15))
                }
            }
            .textSelection(.enabled)
        }
        .frame(maxHeight: Self.height)
        .fixedSize(horizontal: false, vertical: true)
        .clipShape(.rect(cornerRadius: 8))
    }

    private static func lines(_ before: String, _ after: String) -> [(removed: Bool, text: String)] {
        let old = before.isEmpty ? [] : before.components(separatedBy: "\n")
        let new = after.isEmpty ? [] : after.components(separatedBy: "\n")
        var removed: [String] = []
        var added: [String] = []
        for step in new.difference(from: old) {
            switch step {
            case .remove(_, let line, _): removed.append(line)
            case .insert(_, let line, _): added.append(line)
            }
        }
        return removed.map { (true, $0) } + added.map { (false, $0) }
    }
}
