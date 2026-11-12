import ComposableArchitecture
import HumataDaemonClient
import HumataReview
import SwiftUI

/// The sheet to land a session: a branch, a message, and whether to open
/// pull requests; then what landed, with links to them.
struct LandView: View {
    @Bindable var store: StoreOf<LandFeature>

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Land the Session")
                .font(.headline)
            if let landed = store.landed {
                LandedList(landed: landed)
            } else {
                form
            }
            if let error = store.error {
                Text(error)
                    .font(.callout)
                    .foregroundStyle(.red)
                    .textSelection(.enabled)
            }
            HStack {
                Spacer()
                if store.landed == nil {
                    Button("Cancel") { store.send(.doneTapped) }
                        .keyboardShortcut(.cancelAction)
                    Button(store.openPullRequests ? "Land and Open Pull Requests" : "Land") {
                        store.send(.landTapped)
                    }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!store.canLand)
                } else {
                    Button("Done") { store.send(.doneTapped) }
                        .keyboardShortcut(.defaultAction)
                }
            }
        }
        .padding(20)
        .frame(width: 480)
    }

    private var form: some View {
        Form {
            TextField("Branch", text: $store.branch)
                .help("A new branch in each repository the session changed")
            TextField("Message", text: $store.message, axis: .vertical)
                .lineLimit(3...8)
                .help("The commit message; its first line titles the pull requests")
            Toggle("Open pull requests on GitHub", isOn: $store.openPullRequests)
                .help(
                    "Pushes each branch and opens a pull request for it, with This Mac's GitHub token. "
                        + "When they all merge, the task is done and the session's copy is removed.")
            if !store.openPullRequests {
                Toggle("Push the branches", isOn: $store.push)
            }
        }
        .disabled(store.landing)
        .overlay {
            if store.landing { ProgressView() }
        }
    }
}

/// What landed: each repository's branch, and the pull requests opened.
private struct LandedList: View {
    let landed: Landed

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(landed.repositories, id: \.name) { repository in
                Label {
                    Text(
                        "\(repository.name): \(repository.branch) onto \(repository.base)"
                            + (repository.pushed ? ", pushed" : ""))
                } icon: {
                    Image(systemName: repository.commit.isEmpty ? "minus.circle" : "checkmark.circle.fill")
                        .foregroundStyle(repository.commit.isEmpty ? Color.secondary : Color.green)
                }
            }
            ForEach(landed.pullRequests) { pull in
                PullRequestRow(pull: pull)
            }
        }
    }
}
