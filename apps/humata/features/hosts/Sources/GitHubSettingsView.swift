import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// Settings › GitHub.
package struct GitHubSettingsView: View {
    @Bindable var store: StoreOf<GitHubSettingsFeature>

    package init(store: StoreOf<GitHubSettingsFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                Text(
                    """
                    Humata uses the GitHub command line's sign-in (`gh auth login`), or `GH_TOKEN` in \
                    Hosts › This Mac's variables, to open and follow pull requests and to list saved owners' \
                    repositories.
                    """
                )
                .foregroundStyle(.secondary)
                if let source = store.refresh?.tokenSource {
                    Self.token(source)
                }
            } header: {
                Text("Sign-In")
            }
            if let refresh = store.refresh {
                pullRequests(refresh)
            } else if store.failure == nil {
                HStack(spacing: 8) {
                    ProgressView()
                        .controlSize(.small)
                    Text("Reading the settings…")
                        .foregroundStyle(.secondary)
                }
            }
            if let failure = store.failure {
                VStack(alignment: .leading, spacing: 6) {
                    Text(failure)
                        .foregroundStyle(.red)
                    if store.refresh == nil {
                        Button("Try Again") { store.send(.task) }
                    }
                }
            }
        }
        .formStyle(.grouped)
        .task { await store.send(.task).finish() }
    }

    private func pullRequests(_ refresh: PullRefresh) -> some View {
        Section {
            Picker(
                "Read pull requests",
                selection: Binding(get: { refresh.automatic }, set: { store.send(.automaticPicked($0)) })
            ) {
                Text("Automatically").tag(true)
                Text("Only when I ask").tag(false)
            }
            if refresh.automatic {
                Picker(
                    "Every",
                    selection: Binding(get: { store.every }, set: { store.send(.everyPicked($0)) })
                ) {
                    ForEach(GitHubSettingsFeature.State.minuteChoices, id: \.self) { minutes in
                        Text(Self.minutes(minutes)).tag(GitHubSettingsFeature.Every.minutes(minutes))
                    }
                    Divider()
                    Text("Custom…").tag(GitHubSettingsFeature.Every.custom)
                }
                if store.every == .custom {
                    LabeledContent("Minutes") {
                        HStack {
                            TextField("1–1440", text: $store.customMinutes)
                                .labelsHidden()
                                .multilineTextAlignment(.trailing)
                                .frame(width: 80)
                                .onSubmit { store.send(.customMinutesSubmitted) }
                            Button("Set") { store.send(.customMinutesSubmitted) }
                        }
                    }
                }
            }
        } header: {
            HStack(spacing: 8) {
                Text("Pull Requests")
                if store.saving {
                    ProgressView()
                        .controlSize(.small)
                }
            }
        } footer: {
            Text(
                """
                With Only when I ask, a task's pull requests are read when you press Refresh on its page. \
                Every read counts against GitHub's rate limit, so pick a longer interval if you hit the limit \
                or have many open pull requests.
                """
            )
            .foregroundStyle(.secondary)
        }
        .disabled(store.saving)
    }

    /// One line saying where the token comes from, or that there is none;
    /// nothing when the daemon didn't say.
    @ViewBuilder
    private static func token(_ source: GitHubTokenSource) -> some View {
        switch source {
        case .unknown:
            EmptyView()
        case .commandLine:
            Label("Using the GitHub command line's sign-in", systemImage: "checkmark.circle")
        case .variable(let name):
            Label("Using \(name) from This Mac's variables", systemImage: "checkmark.circle")
        case .missing:
            Label(
                "No token: run `gh auth login` on this Mac, or add GH_TOKEN in Hosts › This Mac",
                systemImage: "exclamationmark.triangle"
            )
            .foregroundStyle(.orange)
        }
    }

    private static func minutes(_ minutes: Int) -> String {
        minutes == 1 ? String(localized: "1 minute") : String(localized: "\(minutes) minutes")
    }
}
