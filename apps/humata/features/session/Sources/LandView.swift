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
            if store.landed == nil, !store.landing, let why = store.cantLand {
                Text(why)
                    .font(.callout)
                    .foregroundStyle(.secondary)
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
                        .disabled(store.landing)
                    // ⌘↩, so Return can start a new line of the message.
                    Button(store.openPullRequests ? "Land and Open Pull Requests" : "Land") {
                        store.send(.landTapped)
                    }
                    .keyboardShortcut(.return, modifiers: .command)
                    .buttonStyle(.borderedProminent)
                    .disabled(!store.canLand)
                    .help(
                        store.cantLand
                            ?? (store.landing
                                ? String(localized: "Landing…")
                                : String(
                                    localized: "Commit the work onto the branch, then push it or open pull requests")))
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
            LabeledContent("Message") {
                TextEditor(text: $store.message)
                    .font(.body)
                    .frame(minHeight: 60, maxHeight: 140)
                    .accessibilityLabel("Message")
            }
            .help("The commit message; its first line titles the pull requests. ⌘↩ lands.")
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
