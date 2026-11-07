import ComposableArchitecture
import Foundation

/// The models on this device: the catalog, what's downloaded, downloads in
/// progress, and which model each use runs on. Downloads come only from
/// Hugging Face, pinned to a commit and checked against each file's digest.
@DependencyClient
package struct ModelLibraryClient: Sendable {
    package var catalog: @Sendable () -> [ModelSpec] = { [] }
    /// The models whose files are downloaded and checked.
    package var installed: @Sendable () async -> [InstalledModel] = { [] }
    /// The models downloads were started for and haven't finished, running
    /// or paused, even ones from before Lifr last quit.
    package var unfinished: @Sendable () async -> [ModelSpec] = { [] }
    // A typed throw needs its closure's parameters spelled out, and the
    // formatter puts them on their own line.
    // swiftlint:disable closure_parameter_position
    /// The model a pasted Hugging Face link points at, pinned to a commit,
    /// with each file's digest: the SHA-256 the person gave, which must
    /// match Hugging Face's, or Hugging Face's. Downloads nothing.
    package var resolve: @Sendable (_ link: String, _ sha256: String?) async throws(ModelResolveError) -> ModelSpec = {
        link, _ throws(ModelResolveError) in
        throw .notALink(link)
    }
    /// Downloads the model's files, or what's left of them, checks them and
    /// moves them into place. Progress goes to `downloads`.
    package var download: @Sendable (_ spec: ModelSpec) async throws(ModelDownloadError) -> InstalledModel = {
        _ throws(ModelDownloadError) in
        throw .nothingToDownload
    }
    /// Each download's state, each time one changes. Finished downloads
    /// leave it.
    package var downloads: @Sendable () -> AsyncStream<[ModelID: DownloadState]> = { .finished }
    /// Pauses a download. Downloading again goes on from where it stopped.
    package var pause: @Sendable (_ id: ModelID) async -> Void
    /// Removes the model's files, and any partial download. Uses that ran
    /// on it go back to Apple's model.
    package var delete: @Sendable (_ id: ModelID) async throws(ModelStorageError) -> Void = {
        _ throws(ModelStorageError) in
    }
    // swiftlint:enable closure_parameter_position
    /// The model `use` runs on.
    package var selection: @Sendable (_ use: ModelUse) -> ModelID = { _ in .apple }
    package var select: @Sendable (_ id: ModelID, _ use: ModelUse) async -> Void
    /// Sets the window `id` runs at, one of its `contextChoices`; its own
    /// window, or one it doesn't offer, goes back to its own.
    package var setWindow: @Sendable (_ id: ModelID, _ window: Int) async -> Void
}

extension ModelLibraryClient: TestDependencyKey {
    package static let testValue = ModelLibraryClient()
    package static let previewValue = InMemoryModelLibrary().client
}

extension DependencyValues {
    package var models: ModelLibraryClient {
        get { self[ModelLibraryClient.self] }
        set { self[ModelLibraryClient.self] = newValue }
    }
}

/// A library kept in memory, for tests and previews. Links resolve to the
/// specs in `links`; a download finishes at once, or fails with the error
/// set for it.
package final class InMemoryModelLibrary: Sendable {
    private struct State {
        var installed: [ModelID: InstalledModel] = [:]
        var selection: [ModelUse: ModelID] = [:]
        var failures: [ModelID: ModelDownloadError] = [:]
        var unfinished: [ModelID: ModelSpec] = [:]
    }

    private let state = LockIsolated(State())
    private let board = DownloadBoard()
    private let links: [String: Result<ModelSpec, ModelResolveError>]
    private let catalog: [ModelSpec]
    private let now: Date

    package init(
        catalog: [ModelSpec] = ModelCatalog.all, installed: [ModelSpec] = [],
        links: [String: Result<ModelSpec, ModelResolveError>] = [:],
        now: Date = Date(timeIntervalSince1970: 1_793_700_000)
    ) {
        self.catalog = catalog
        self.links = links
        self.now = now
        state.withValue { state in
            for spec in installed { state.installed[spec.id] = Self.installed(spec, at: now) }
        }
    }

    /// The models downloaded so far.
    package var installed: [ModelID] { state.value.installed.keys.sorted { $0.rawValue < $1.rawValue } }

    /// Makes the next download of `id` fail with `error`.
    package func failNextDownload(of id: ModelID, with error: ModelDownloadError) {
        state.withValue { $0.failures[id] = error }
    }

    /// Keeps `spec`'s download unfinished, as one paused before Lifr quit.
    package func leaveUnfinished(_ spec: ModelSpec, received: Int64) {
        state.withValue { $0.unfinished[spec.id] = spec }
        board.update { $0[spec.id] = .paused(received: received, total: spec.source?.totalSize ?? 0) }
    }

    /// Shows `download` for `id`, as a download in progress would.
    package func show(_ download: DownloadState?, for id: ModelID) {
        board.update { $0[id] = download }
    }

    package var client: ModelLibraryClient {
        ModelLibraryClient(
            catalog: { [catalog] in catalog },
            installed: { [state] in
                state.value.installed.values.sorted { $0.id.rawValue < $1.id.rawValue }
            },
            unfinished: { [state] in
                state.value.unfinished.values.sorted { $0.id.rawValue < $1.id.rawValue }
            },
            resolve: { [links] link, _ throws(ModelResolveError) in
                guard let result = links[link] else { throw .notALink(link) }
                return try result.get()
            },
            download: { [self] spec throws(ModelDownloadError) in
                if let error = state.withValue({ $0.failures.removeValue(forKey: spec.id) }) {
                    board.update { $0[spec.id] = .failed(error) }
                    throw error
                }
                let model = Self.installed(spec, at: now)
                state.withValue { state in
                    state.installed[spec.id] = model
                    state.unfinished[spec.id] = nil
                }
                board.update { $0[spec.id] = nil }
                return model
            },
            downloads: { [board] in board.stream() },
            pause: { [board] id in
                board.update { downloads in
                    if case .downloading(let received, let total) = downloads[id] {
                        downloads[id] = .paused(received: received, total: total)
                    }
                }
            },
            delete: { [self] id throws(ModelStorageError) in
                state.withValue { state in
                    state.installed[id] = nil
                    state.unfinished[id] = nil
                    for (use, selected) in state.selection where selected == id { state.selection[use] = nil }
                }
                board.update { $0[id] = nil }
            },
            selection: { [state] use in use.model(in: state.value.selection) },
            select: { [state] id, use in state.withValue { $0.selection[use] = id } },
            setWindow: { [state] id, window in
                state.withValue { state in
                    guard let spec = state.installed[id]?.spec else { return }
                    state.installed[id]?.chosenWindow = ModelSpec.choice(window, of: spec)
                }
            }
        )
    }

    private static func installed(_ spec: ModelSpec, at now: Date) -> InstalledModel {
        InstalledModel(
            spec: spec, directory: URL(filePath: "/models/\(spec.id.directoryName)", directoryHint: .isDirectory),
            installedAt: now)
    }
}
