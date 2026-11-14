import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// Settings › Repositories.
package struct SavedRepositoriesView: View {
    @Bindable var store: StoreOf<SavedRepositoriesFeature>

    package init(store: StoreOf<SavedRepositoriesFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            if let failure = store.failure {
                Section {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(failure)
                            .foregroundStyle(.red)
                        if store.repositories == nil {
                            Button("Try Again") { store.send(.task) }
                        }
                    }
                }
            }
            owners
            repositories
        }
        .formStyle(.grouped)
        .task { await store.send(.task).finish() }
    }

    @ViewBuilder private var owners: some View {
        Section {
            if let owners = store.owners {
                ForEach(owners) { owner in
                    OwnerRow(
                        owner: owner, listing: store.listing.contains(owner.login),
                        refreshing: store.refreshing.contains(owner.login) || store.refreshing.contains(""),
                        refresh: { store.send(.refreshTapped(owner.login)) },
                        remove: { store.send(.removeOwnerTapped(owner.login)) })
                }
            } else if store.loading {
                loading
            }
            HStack {
                TextField("Owner", text: $store.newOwner, prompt: Text("octo-org, @octo-org or its GitHub page"))
                    .labelsHidden()
                    .onSubmit { store.send(.addOwnerTapped) }
                if store.addingOwner {
                    ProgressView().controlSize(.small)
                }
                Button("Add") { store.send(.addOwnerTapped) }
                    .disabled(!store.canAddOwner)
            }
        } header: {
            HStack(spacing: 8) {
                Text("GitHub Owners")
                Spacer()
                if store.refreshing.contains("") {
                    ProgressView().controlSize(.small)
                }
                Button("Refresh All") { store.send(.refreshTapped(nil)) }
                    .disabled((store.owners?.isEmpty ?? true) || !store.refreshing.isEmpty)
            }
        } footer: {
            Text(
                """
                The repositories of these GitHub users and organizations are offered when you make or change a \
                project, listed with this Mac's GitHub token: an organization's private ones too, and a user's \
                own private ones for the token's user. They're listed again every 6 hours; each listing counts \
                against GitHub's rate limit.
                """
            )
            .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var repositories: some View {
        Section {
            if let repositories = store.repositories {
                if repositories.isEmpty {
                    Text("None yet. Repositories are saved as projects are made with them, and as owners are listed.")
                        .foregroundStyle(.secondary)
                }
                ForEach(repositories) { repository in
                    SavedRepositoryRow(store: store, repository: repository)
                }
            } else if store.loading {
                loading
            }
        } header: {
            Text("Saved Repositories")
        } footer: {
            Text(
                "Offered first when you make or change a project, last used first. Renaming or removing one "
                    + "changes no project; one removed from GitHub stays out when its owner is listed again."
            )
            .foregroundStyle(.secondary)
        }
    }

    private var loading: some View {
        HStack(spacing: 8) {
            ProgressView().controlSize(.small)
            Text("Reading…").foregroundStyle(.secondary)
        }
    }
}

private struct OwnerRow: View {
    let owner: RepositoryOwner
    let listing: Bool
    let refreshing: Bool
    let refresh: () -> Void
    let remove: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(owner.login).fontWeight(.medium)
                    Text(summary)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if listing || refreshing {
                    ProgressView().controlSize(.small)
                }
                Button("Refresh", action: refresh)
                    .disabled(refreshing)
                Button(role: .destructive, action: remove) {
                    Label("Remove", systemImage: "minus.circle")
                        .labelStyle(.iconOnly)
                }
                .buttonStyle(.borderless)
                .help("Stop offering this owner's repositories; those a project used stay")
            }
            if let error = owner.error {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        }
    }

    private var summary: String {
        let count =
            owner.repositories == 1
            ? String(localized: "1 repository") : String(localized: "\(owner.repositories) repositories")
        guard let refreshed = owner.refreshed else {
            return listing ? String(localized: "Listing its repositories…") : count
        }
        return String(localized: "\(count), listed \(refreshed.formatted(.relative(presentation: .named)))")
    }
}

private struct SavedRepositoryRow: View {
    @Bindable var store: StoreOf<SavedRepositoriesFeature>
    let repository: SavedRepository

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                if store.renaming == repository.id {
                    TextField("Name", text: $store.newName)
                        .labelsHidden()
                        .onSubmit { store.send(.renameSubmitted) }
                        .onExitCommand { store.send(.renameCancelled) }
                } else {
                    Text(repository.name).fontWeight(.medium)
                }
                Text(repository.location)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            if store.renaming == repository.id {
                Button("Save") { store.send(.renameSubmitted) }
                Button("Cancel") { store.send(.renameCancelled) }
            } else {
                Button("Rename") { store.send(.renameTapped(repository.id)) }
            }
            Button(role: .destructive) {
                store.send(.removeTapped(repository.id))
            } label: {
                Label("Remove", systemImage: "minus.circle")
                    .labelStyle(.iconOnly)
            }
            .buttonStyle(.borderless)
            .help("Stop offering this repository; projects that have it keep it")
        }
    }

    private var detail: String {
        let source =
            switch repository.source {
            case .github: String(localized: "From GitHub, listed for \(repository.owner)")
            case .used: String(localized: "Used by a project")
            }
        guard let used = repository.lastUsed else { return source }
        return String(localized: "\(source) · last used \(used.formatted(.relative(presentation: .named)))")
    }
}
