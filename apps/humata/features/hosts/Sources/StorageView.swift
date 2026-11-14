import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// Settings › Storage.
package struct StorageView: View {
    @Bindable var store: StoreOf<StorageFeature>

    package init(store: StoreOf<StorageFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            if let storage = store.storage {
                Section {
                    ForEach(uses(storage)) { use in
                        UseRow(use: use) { store.send(.showInFinderTapped(use.path)) }
                    }
                    LabeledContent("Total") {
                        Text(Self.size(store.total))
                            .monospacedDigit()
                            .bold()
                    }
                } header: {
                    Text("On This Mac")
                }
                cleanUp(storage)
                if !store.models.isEmpty {
                    models
                }
                automatically(storage)
            } else if store.failure == nil {
                HStack(spacing: 8) {
                    ProgressView()
                        .controlSize(.small)
                    Text("Measuring Humata's files…")
                        .foregroundStyle(.secondary)
                }
            }
            // What the person asked for and wasn't done, kept through a
            // cleanup's progress reads.
            if let failure = store.actionFailure {
                Text(failure)
                    .foregroundStyle(.red)
                    .textSelection(.enabled)
            }
            if let failure = store.failure {
                Text(failure)
                    .foregroundStyle(.red)
                // A cleanup under way reads its progress again by itself.
                if store.storage?.job == nil {
                    Button("Try Again") { store.send(.task) }
                }
            }
        }
        .formStyle(.grouped)
        .alert($store.scope(state: \.alert, action: \.alert))
        .task { await store.send(.task).finish() }
    }

    /// humatad's files, then the models, if any.
    private func uses(_ storage: Storage) -> [Storage.Use] {
        guard !store.models.isEmpty else { return storage.uses }
        return storage.uses + [Storage.Use(kind: .models, bytes: store.modelsBytes, path: store.modelsFolder)]
    }

    private func cleanUp(_ storage: Storage) -> some View {
        Section {
            if let job = storage.job {
                JobRow(job: job)
            }
            Group {
                CleanupRow(
                    title: "Archived sessions not used in \(storage.days) days",
                    detail: "Their histories, snapshots and branches",
                    button: "Delete…"
                ) { store.send(.deleteArchivedTapped) }
                CleanupRow(
                    title: "Archived sessions' working copies",
                    detail: "Made again when a session is unarchived",
                    bytes: storage.archivedWorktreesBytes,
                    button: "Remove…"
                ) { store.send(.putAwayWorktreesTapped) }
                CleanupRow(
                    title: "Repositories no project uses",
                    detail: "Clones left from projects that were removed",
                    bytes: storage.unusedClonesBytes,
                    button: "Delete…"
                ) { store.send(.removeUnusedClonesTapped) }
                CleanupRow(
                    title: "Compact",
                    detail: "Packs repositories, drops mid-turn saves, compacts the store; nothing you can open goes",
                    button: "Compact"
                ) { store.send(.compactTapped) }
            }
            .disabled(storage.job != nil)
            if let cleaned = storage.cleaned {
                LastCleanup(storage: storage, cleaned: cleaned)
            }
        } header: {
            Text("Clean Up")
        }
    }

    private var models: some View {
        Section {
            ForEach(store.models) { model in
                LabeledContent {
                    HStack {
                        Text(Self.size(model.bytes))
                            .monospacedDigit()
                        if store.deletingModel == model.id {
                            ProgressView()
                                .controlSize(.small)
                        } else {
                            Button("Delete…") { store.send(.deleteModelTapped(model.id)) }
                                .disabled(store.deletingModel != nil)
                        }
                    }
                } label: {
                    Text(model.name)
                }
            }
        } header: {
            Text("Downloaded Models")
        }
    }

    private func automatically(_ storage: Storage) -> some View {
        Section {
            Toggle(
                "Delete archived sessions automatically",
                isOn: Binding(
                    get: { storage.autoDelete },
                    set: { store.send(.autoDeleteToggled($0)) }))
            Picker(
                "Keep archived sessions for",
                selection: Binding(get: { storage.days }, set: { store.send(.daysPicked($0)) })
            ) {
                ForEach(Self.choices(storage.days), id: \.self) { days in
                    Text("\(days) days").tag(days)
                }
            }
        } header: {
            Text("Automatically")
        } footer: {
            Text(
                """
                Once a day, while no session works. Sessions that aren't archived are never deleted, nor \
                keepers, sessions with forks, sessions with commits nothing else holds, or sessions on a host \
                that can't be reached; those are listed after each cleanup.
                """
            )
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }

    /// The offered choices, and the one set if it isn't among them.
    private static func choices(_ days: Int) -> [Int] {
        StorageFeature.State.dayChoices.contains(days)
            ? StorageFeature.State.dayChoices
            : (StorageFeature.State.dayChoices + [days]).sorted()
    }

    static func size(_ bytes: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(clamping: bytes), countStyle: .file)
    }
}

private struct UseRow: View {
    var use: Storage.Use
    var show: () -> Void

    var body: some View {
        LabeledContent {
            HStack {
                Text(StorageView.size(use.bytes))
                    .monospacedDigit()
                Button {
                    show()
                } label: {
                    Image(systemName: "folder")
                }
                .buttonStyle(.borderless)
                .help("Show in Finder")
                .accessibilityLabel("Show \(title) in Finder")
            }
        } label: {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var title: String {
        switch use.kind {
        case .clones: "Repositories"
        case .worktrees: "Working Copies"
        case .histories: "Session Histories"
        case .recovery: "Recovery Copies"
        case .store: "Projects and Board"
        case .search: "Search Index"
        case .logs: "Logs"
        case .models: "Models"
        }
    }

    private var detail: String {
        switch use.kind {
        case .clones: "Cloned by URL, with sessions' branches and snapshots"
        case .worktrees: "Each session's copy of its code"
        case .histories: "What each session said and did"
        case .recovery: "Copies of other machines' sessions, to recover them"
        case .store: "Projects, tasks and reviews"
        case .search: "What Search looks in"
        case .logs: "The host's log, and the one before"
        case .models: "Downloaded for local-model sessions"
        }
    }
}

/// A cleanup by hand: what it removes, how much when known, and its button.
private struct CleanupRow: View {
    var title: String
    var detail: String
    var bytes: UInt64?
    var button: String
    var action: () -> Void

    var body: some View {
        LabeledContent {
            HStack {
                if let bytes {
                    Text(StorageView.size(bytes))
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                }
                Button(button, action: action)
                    .disabled(bytes == 0)
            }
        } label: {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

private struct JobRow: View {
    var job: Storage.Job

    var body: some View {
        if job.total > 0 {
            ProgressView(value: Double(job.done), total: Double(job.total)) {
                Text(job.step)
            } currentValueLabel: {
                Text("\(job.done) of \(job.total)")
                    .monospacedDigit()
            }
        } else {
            HStack(spacing: 8) {
                ProgressView()
                    .controlSize(.small)
                Text(job.step)
            }
        }
    }
}

private struct LastCleanup: View {
    var storage: Storage
    var cleaned: Date

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(summary)
                .foregroundStyle(.secondary)
            if let failure = storage.failure {
                Text(failure)
                    .foregroundStyle(.red)
            }
            ForEach(storage.skipped) { skip in
                VStack(alignment: .leading, spacing: 1) {
                    Text(skip.name.isEmpty ? "A session" : skip.name)
                    Text(skip.reason)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private var summary: String {
        let when = cleaned.formatted(date: .abbreviated, time: .shortened)
        let deleted =
            switch storage.deletedSessions {
            case 0: "no sessions deleted"
            case 1: "1 session deleted"
            default: "\(storage.deletedSessions) sessions deleted"
            }
        let kept = storage.skipped.isEmpty ? "" : "; \(storage.skipped.count) kept:"
        return "Last cleaned \(when): \(deleted)\(kept)"
    }
}
