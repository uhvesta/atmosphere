import HumataDaemonClient
import SwiftUI

/// A pull request with where it stands, linked to GitHub.
package struct PullRequestRow: View {
    package let pull: PullRequest

    package init(pull: PullRequest) {
        self.pull = pull
    }

    package var body: some View {
        HStack(spacing: 8) {
            Image(systemName: symbol)
                .foregroundStyle(color)
            VStack(alignment: .leading, spacing: 1) {
                if let url = URL(string: pull.url) {
                    Link("\(pull.githubRepository)#\(pull.number)", destination: url)
                        .font(.callout.bold())
                } else {
                    Text("\(pull.githubRepository)#\(pull.number)").font(.callout.bold())
                }
                Text(pull.title)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                if let problem = pull.problem {
                    Label(problem, systemImage: "exclamationmark.triangle")
                        .font(.caption)
                        .foregroundStyle(.orange)
                        .lineLimit(3)
                        .textSelection(.enabled)
                        .help("Shown as last read from GitHub. It's read again every few minutes.")
                }
            }
            Spacer()
            Text(standing)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var symbol: String {
        switch pull.state {
        case .open: "arrow.triangle.pull"
        case .merged: "arrow.triangle.merge"
        case .closed: "xmark.circle"
        }
    }

    private var color: Color {
        switch pull.state {
        case .open: .green
        case .merged: .purple
        case .closed: .secondary
        }
    }

    private var standing: String {
        if pull.state != .open { return pull.state == .merged ? "Merged" : "Closed" }
        let review =
            switch pull.review {
            case .none: "No review yet"
            case .changesRequested: "Changes requested"
            case .commented: "Commented"
            case .approved: "Approved"
            }
        let checks =
            switch pull.checks {
            case .none: ""
            case .pending: " · checks running"
            case .passing: " · checks pass"
            case .failing: " · checks fail"
            }
        return review + checks
    }
}
