import CryptoKit
import Foundation
import InferenceKit

/// The models on this device, in `root`: `{id}/{commit}` for each one
/// downloaded, with a manifest, and `.staging/{id}/{commit}` while it
/// downloads. A model moves into place only once every file matches its
/// digest, so a model in `root` is always whole.
public final class ModelLibrary: Sendable {
    private struct State {
        var downloads: [ModelID: Task<Result<InstalledModel, ModelDownloadError>, Never>] = [:]
        /// The window picked for each model that has a choice.
        var windows: [ModelID: Int] = [:]
        /// Where each model runs and how fast it ran, for those the person
        /// tuned.
        var tuning: [ModelID: Tuning] = [:]
    }

    /// What's kept of a model's tuning: where the person picked for it to
    /// run, its measurements, how each setup compared when the person
    /// tuned it, and the setup they picked for each purpose.
    private struct Tuning: Codable {
        var picked: ModelBackend?
        var measurements: [ModelMeasurement] = []
        var results: [TuneResult] = []
        var setups: [TunePurpose: TunedSetup] = [:]

        init() {}

        private enum CodingKeys: String, CodingKey {
            case picked, measurements, results, setups
        }

        /// Reads tuning kept before setups were compared as having none.
        init(from decoder: any Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            picked = try container.decodeIfPresent(ModelBackend.self, forKey: .picked)
            measurements = try container.decodeIfPresent([ModelMeasurement].self, forKey: .measurements) ?? []
            results = try container.decodeIfPresent([TuneResult].self, forKey: .results) ?? []
            setups = try container.decodeIfPresent([TunePurpose: TunedSetup].self, forKey: .setups) ?? [:]
        }
    }

    private let root: URL
    private let hub: HubClient
    private let fetcher: any ModelFetcher
    private let capacity: @Sendable (URL) throws -> Int64
    /// The models offered without a link.
    public let catalog: [ModelSpec]
    private let now: @Sendable () -> Date
    private let state = Locked(State())
    private let board = DownloadBoard()

    /// - Parameters:
    ///   - root: Where the models go. Created, and kept out of backups.
    ///   - capacity: The bytes free for important use on `root`'s volume.
    ///   - chunkSize: The bytes asked for at once. A pause or a dropped
    ///     connection loses at most this much.
    public convenience init(
        root: URL, session: URLSession, capacity: @escaping @Sendable (URL) throws -> Int64 = ModelLibrary.capacity,
        chunkSize: Int64 = 32 << 20, catalog: [ModelSpec] = ModelCatalog.all,
        now: @escaping @Sendable () -> Date = Date.init
    ) {
        let hub = HubClient(session: session)
        self.init(
            root: root, hub: hub, fetcher: ChunkedFetcher(hub: hub, chunkSize: chunkSize), capacity: capacity,
            catalog: catalog, now: now)
    }

    /// - Parameter fetcher: What brings each file's bytes down.
    init(
        root: URL, hub: HubClient, fetcher: any ModelFetcher, capacity: @escaping @Sendable (URL) throws -> Int64,
        catalog: [ModelSpec], now: @escaping @Sendable () -> Date
    ) {
        self.root = root
        self.hub = hub
        self.fetcher = fetcher
        self.capacity = capacity
        self.catalog = catalog
        self.now = now
        let windows = try? JSONDecoder().decode([ModelID: Int].self, from: Data(contentsOf: windowsFile))
        let tuning = try? JSONDecoder().decode([ModelID: Tuning].self, from: Data(contentsOf: tuningFile))
        state.withValue {
            $0.windows = windows ?? [:]
            $0.tuning = tuning ?? [:]
        }
    }

    /// The library in Application Support, in a folder for the app on the
    /// Mac. On the iPhone, files download in the background, so they go on
    /// while the app is away.
    /// - Parameters:
    ///   - root: Where the models go; Application Support's Models folder,
    ///     in the app's own folder on the Mac, by default.
    ///   - userAgent: What requests to Hugging Face say they come from,
    ///     such as the app's name.
    public static func live(
        root: URL = ModelLibrary.defaultRoot, userAgent: String, catalog: [ModelSpec] = ModelCatalog.all
    ) -> ModelLibrary {
        let configuration = URLSessionConfiguration.default
        configuration.httpAdditionalHeaders = ["User-Agent": userAgent]
        let hub = HubClient(session: URLSession(configuration: configuration))
        #if os(macOS)
            let fetcher: any ModelFetcher = ChunkedFetcher(hub: hub, chunkSize: 32 << 20)
        #else
            let fetcher: any ModelFetcher = backgroundFetcher(root: root, userAgent: userAgent)
        #endif
        return ModelLibrary(
            root: root, hub: hub, fetcher: fetcher, capacity: ModelLibrary.capacity, catalog: catalog,
            now: Date.init)
    }

    /// Application Support's Models folder: in a folder named for the app
    /// on the Mac, where Application Support is shared.
    public static var defaultRoot: URL {
        let support = URL.applicationSupportDirectory
        #if os(macOS)
            let app = Bundle.main.bundleIdentifier ?? "InferenceKit"
            return support.appending(path: "\(app)/Models", directoryHint: .isDirectory)
        #else
            return support.appending(path: "Models", directoryHint: .isDirectory)
        #endif
    }

    /// The one background session there may be, for the library's root.
    private static let background = Locked<BackgroundFetcher?>(nil)

    private static func backgroundFetcher(root: URL, userAgent: String) -> BackgroundFetcher {
        background.withValue { fetcher in
            if let fetcher { return fetcher }
            let made = BackgroundFetcher(root: root, userAgent: userAgent)
            fetcher = made
            return made
        }
    }

    /// Returns once the system has handed the app the downloads it finished
    /// while the app wasn't running, for SwiftUI's
    /// `.backgroundTask(.urlSession(backgroundSession))`.
    public static func caughtUp() async {
        await background.value?.caughtUp()
    }

    /// The background session's identifier.
    public static var backgroundSession: String { BackgroundFetcher.identifier }

    /// The bytes free for important use on `url`'s volume.
    public static func capacity(_ url: URL) throws -> Int64 {
        var existing = url
        while !FileManager.default.fileExists(atPath: existing.path(percentEncoded: false)),
            existing.pathComponents.count > 1
        {
            existing = existing.deletingLastPathComponent()
        }
        let values = try existing.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey])
        return values.volumeAvailableCapacityForImportantUsage ?? 0
    }

    /// Each download's state, now and each time one changes. Finished
    /// downloads leave it.
    public func downloads() -> AsyncStream<[ModelID: DownloadState]> { board.stream() }

    /// Pauses a download. Downloading again goes on from where it stopped.
    public func pause(_ id: ModelID) async { await stop(id) }

    /// The models downloads were started for and haven't finished, running
    /// or paused, even ones from before the app last quit.
    public func unfinishedModels() -> [ModelSpec] { unfinished().map(\.spec) }

    // MARK: Installed models

    /// The models whose files are in place.
    public func installed() -> [InstalledModel] {
        let manager = FileManager.default
        let folders = (try? manager.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? []
        let (windows, tuning) = state.withValue { ($0.windows, $0.tuning) }
        return folders.filter { !$0.lastPathComponent.hasPrefix(".") }.flatMap { model in
            let commits = (try? manager.contentsOfDirectory(at: model, includingPropertiesForKeys: nil)) ?? []
            return commits.compactMap(Self.manifest)
        }
        .map { model in
            var model = model
            model.chosenWindow = windows[model.id].flatMap { ModelSpec.choice($0, of: model.spec) }
            model.pickedBackend = tuning[model.id]?.picked
            model.measurements = tuning[model.id]?.measurements ?? []
            return model
        }
        .sorted { $0.installedAt < $1.installedAt }
    }

    /// The model in `directory`, if its manifest is there.
    private static func manifest(in directory: URL) -> InstalledModel? {
        let file = directory.appending(path: Manifest.name, directoryHint: .notDirectory)
        guard let data = try? Data(contentsOf: file),
            let manifest = try? JSONDecoder().decode(Manifest.self, from: data)
        else { return nil }
        var spec = manifest.spec
        if let entry = ModelCatalog.entry(for: spec) {
            spec.inputs.formUnion(entry.inputs)
            spec.memoryBytes = entry.memoryBytes
            spec.cpuMemoryBytes = entry.cpuMemoryBytes
            spec.contextChoices = entry.contextChoices
            spec.memoryBytesPerToken = entry.memoryBytesPerToken
            spec.profile = entry.profile
            spec.variants = entry.variants
        }
        return InstalledModel(spec: spec, directory: directory, installedAt: manifest.installedAt)
    }

    // MARK: Resolving links

    public func resolve(_ text: String, sha256: String?) async throws(ModelResolveError) -> ModelSpec {
        let link = try ModelLink.parse(text)
        let given = try sha256.flatMap { text throws(ModelResolveError) in
            text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                ? nil : try ModelResolution.checkedSHA256(text)
        }
        do {
            let info = try await hub.info(link.repo, revision: link.revision ?? "main")
            guard !info.isGated else { throw ModelResolveError.gated(repo: link.repo) }
            let tree = try await hub.tree(link.repo, commit: info.sha)
            return try ModelResolution.spec(link, info: info, tree: tree, sha256: given, catalog: catalog)
        } catch let error as ModelResolveError {
            throw error
        } catch let error as HubError {
            throw .hub(error)
        } catch {
            throw .hub(.unreadable(link.repo))
        }
    }

    // MARK: Downloads

    /// The downloads started and not finished, with whether each was
    /// stopped, by the person or by an error, rather than by the app quitting.
    func unfinished() -> [Pending] {
        let staging = root.appending(path: ".staging", directoryHint: .isDirectory)
        let folders = (try? FileManager.default.contentsOfDirectory(at: staging, includingPropertiesForKeys: nil)) ?? []
        return folders.compactMap { folder in
            let file = folder.appending(path: Pending.name, directoryHint: .notDirectory)
            guard let data = try? Data(contentsOf: file) else { return nil }
            return try? JSONDecoder().decode(Pending.self, from: data)
        }
        .sorted { $0.spec.id.rawValue < $1.spec.id.rawValue }
    }

    /// Starts again the downloads the app was running when it quit, and shows
    /// the stopped ones as paused.
    public func resumeUnfinished() {
        for pending in unfinished() {
            if pending.stopped {
                let received = received(pending.spec)
                let total = pending.spec.source?.totalSize ?? 0
                board.update { $0[pending.spec.id] = .paused(received: received, total: total) }
            } else {
                Task { [self] in _ = try? await download(pending.spec) }
            }
        }
    }

    /// Marks `spec`'s download as unfinished, and whether it was stopped.
    private func mark(_ spec: ModelSpec, stopped: Bool) {
        let folder = root.appending(path: ".staging/\(spec.id.directoryName)", directoryHint: .isDirectory)
        guard FileManager.default.fileExists(atPath: folder.path(percentEncoded: false)) else { return }
        let file = folder.appending(path: Pending.name, directoryHint: .notDirectory)
        try? JSONEncoder().encode(Pending(spec: spec, stopped: stopped)).write(to: file, options: .atomic)
    }

    /// The bytes of `spec` on the device so far.
    private func received(_ spec: ModelSpec) -> Int64 {
        guard let source = spec.source else { return 0 }
        let staging = root.appending(
            path: ".staging/\(spec.id.directoryName)/\(source.commit)", directoryHint: .isDirectory)
        return source.files.reduce(Int64(0)) { sum, file in
            let done = staging.appending(path: file.path, directoryHint: .notDirectory)
            return sum + (FetchedFile.size(of: done) ?? FetchedFile.size(of: Self.partial(done)) ?? 0)
        }
    }

    public func download(_ spec: ModelSpec) async throws(ModelDownloadError) -> InstalledModel {
        let task = state.withValue { state in
            if let running = state.downloads[spec.id] { return running }
            let task = Task { [self] in
                do throws(ModelDownloadError) {
                    let model = try await fetch(spec)
                    board.update { $0[spec.id] = nil }
                    return Result<InstalledModel, ModelDownloadError>.success(model)
                } catch {
                    let stopped = Task.isCancelled ? ModelDownloadError.paused : error
                    mark(spec, stopped: true)
                    board.update { downloads in
                        if stopped == .paused, case .downloading(let received, let total) = downloads[spec.id] {
                            downloads[spec.id] = .paused(received: received, total: total)
                        } else {
                            downloads[spec.id] = .failed(stopped)
                        }
                    }
                    return .failure(stopped)
                }
            }
            state.downloads[spec.id] = task
            return task
        }
        let result = await task.value
        state.withValue { state in
            if state.downloads[spec.id] == task { state.downloads[spec.id] = nil }
        }
        return try result.get()
    }

    /// Stops the download of `id`, if one is running, and waits for it to.
    private func stop(_ id: ModelID) async {
        guard let task = state.value.downloads[id] else { return }
        task.cancel()
        _ = await task.value
    }

    private func fetch(_ spec: ModelSpec) async throws(ModelDownloadError) -> InstalledModel {
        guard let source = spec.source, !source.files.isEmpty else { throw .nothingToDownload }
        let destination = folder(spec.id).appending(path: source.commit, directoryHint: .isDirectory)
        if let installed = Self.manifest(in: destination) { return installed }
        let staging = root.appending(
            path: ".staging/\(spec.id.directoryName)/\(source.commit)", directoryHint: .isDirectory)
        try storage("create", staging) { try prepare(staging) }
        mark(spec, stopped: false)

        let total = source.totalSize
        let received = Locked(received(spec))
        let available: Int64
        do {
            available = try capacity(root)
        } catch {
            throw .storage(.fileSystem(operation: "check the free space for", path: root.path(), reason: "\(error)"))
        }
        let needed = spaceNeeded(toDownload: total - received.value)
        guard available >= needed else { throw .notEnoughSpace(needed: needed, available: available) }
        board.update { $0[spec.id] = .downloading(received: received.value, total: total) }

        for file in source.files {
            let done = staging.appending(path: file.path, directoryHint: .notDirectory)
            guard FetchedFile.size(of: done) == nil else { continue }
            guard let url = source.url(of: file) else { throw .hub(.notFound(file.path)) }
            let partial = Self.partial(done)
            try storage("create", done.deletingLastPathComponent()) {
                try FileManager.default.createDirectory(
                    at: done.deletingLastPathComponent(), withIntermediateDirectories: true)
            }
            try await fetcher.fetch(file, from: url, into: partial) { [board] bytes in
                let now = received.withValue { received in
                    received += bytes
                    return received
                }
                board.update { $0[spec.id] = .downloading(received: now, total: total) }
            }
            board.update { $0[spec.id] = .verifying }
            guard try Self.matches(partial, file.digest) else {
                try? FileManager.default.removeItem(at: partial)
                throw .digestMismatch(file: file.name)
            }
            try storage("move", partial) { try FileManager.default.moveItem(at: partial, to: done) }
        }

        let manifest = Manifest(spec: spec, installedAt: now())
        let manifestFile = staging.appending(path: Manifest.name, directoryHint: .notDirectory)
        try storage("write", manifestFile) {
            try JSONEncoder().encode(manifest).write(to: manifestFile, options: .atomic)
        }
        try storage("move", staging) {
            try FileManager.default.createDirectory(at: folder(spec.id), withIntermediateDirectories: true)
            try FileManager.default.moveItem(at: staging, to: destination)
            try? FileManager.default.removeItem(at: root.appending(path: ".staging/\(spec.id.directoryName)"))
        }
        return InstalledModel(spec: spec, directory: destination, installedAt: manifest.installedAt)
    }

    /// Whether the bytes in `url` have `digest`, read 8 MB at a time.
    static func matches(_ url: URL, _ digest: FileDigest) throws(ModelDownloadError) -> Bool {
        do {
            let handle = try FileHandle(forReadingFrom: url)
            defer { try? handle.close() }
            switch digest {
            case .sha256(let expected):
                var hash = SHA256()
                try FetchedFile.forEachChunk(of: handle) { hash.update(data: $0) }
                return hex(hash.finalize()) == expected.lowercased()
            case .gitBlobSHA1(let expected):
                let size = try handle.seekToEnd()
                try handle.seek(toOffset: 0)
                var hash = Insecure.SHA1()
                hash.update(data: Data("blob \(size)\0".utf8))
                try FetchedFile.forEachChunk(of: handle) { hash.update(data: $0) }
                return hex(hash.finalize()) == expected.lowercased()
            }
        } catch {
            throw .storage(.fileSystem(operation: "read", path: url.path(), reason: "\(error)"))
        }
    }

    private static func hex(_ digest: some Digest) -> String {
        digest.map { String(format: "%02x", $0) }.joined()
    }

    // MARK: Deleting

    public func delete(_ id: ModelID) async throws(ModelStorageError) {
        await stop(id)
        let staging = root.appending(path: ".staging/\(id.directoryName)", directoryHint: .isDirectory)
        for folder in [folder(id), staging] where FileManager.default.fileExists(atPath: folder.path()) {
            do {
                try FileManager.default.removeItem(at: folder)
            } catch {
                throw .fileSystem(operation: "remove", path: folder.path(), reason: "\(error)")
            }
        }
        board.update { $0[id] = nil }
        saveTuning { $0[id] = nil }
    }

    // MARK: Tuning

    /// Runs `id` on `backend` while the app is on screen, or where the
    /// planner picks with nil.
    public func pick(_ backend: ModelBackend?, for id: ModelID) {
        saveTuning { $0[id, default: Tuning()].picked = backend }
    }

    /// Keeps `measurements` of `id`, in place of earlier ones of the same
    /// backend, variant, window, device and system.
    public func record(_ measurements: [ModelMeasurement], for id: ModelID) {
        saveTuning { tuning in
            var kept = tuning[id, default: Tuning()]
            kept.measurements.removeAll { old in
                measurements.contains {
                    $0.backend == old.backend && $0.runtime == old.runtime && $0.quantization == old.quantization
                        && $0.window == old.window && $0.device == old.device && $0.system == old.system
                }
            }
            kept.measurements += measurements
            tuning[id] = kept
        }
    }

    /// Keeps `results` of `id`'s setups, in place of earlier ones under
    /// the same key.
    public func record(_ results: [TuneResult], for id: ModelID) {
        saveTuning { tuning in
            var kept = tuning[id, default: Tuning()]
            kept.results.removeAll { old in results.contains { $0.key == old.key } }
            kept.results += results
            tuning[id] = kept
        }
    }

    /// How each of `id`'s setups compared, each time the person tuned it,
    /// on any device.
    public func tuneResults(for id: ModelID) -> [TuneResult] {
        state.withValue { $0.tuning[id]?.results ?? [] }
    }

    /// Runs `id` as `setup` for `purpose`, or as the planner picks with nil.
    public func pick(_ setup: TunedSetup?, for purpose: TunePurpose, of id: ModelID) {
        saveTuning { $0[id, default: Tuning()].setups[purpose] = setup }
    }

    /// The setup the person picked for `id` for each purpose.
    public func setups(for id: ModelID) -> [TunePurpose: TunedSetup] {
        state.withValue { $0.tuning[id]?.setups ?? [:] }
    }

    private func saveTuning(_ change: (inout [ModelID: Tuning]) -> Void) {
        let tuning = state.withValue { state in
            change(&state.tuning)
            return state.tuning
        }
        try? prepare(root)
        try? JSONEncoder().encode(tuning).write(to: tuningFile, options: .atomic)
    }

    // MARK: Windows

    /// Runs `id` at `window` from now on; one of its `contextChoices`; its own window, or one
    /// it doesn't offer, goes back to its own.
    public func setWindow(_ window: Int, for id: ModelID) {
        let spec = installed().first { $0.id == id }?.spec
        let windows = state.withValue { state in
            state.windows[id] = spec.flatMap { ModelSpec.choice(window, of: $0) }
            return state.windows
        }
        try? prepare(root)
        try? JSONEncoder().encode(windows).write(to: windowsFile, options: .atomic)
    }

    // MARK: Files

    /// Where `ModelRuntimes` keeps what each use runs on, by default.
    public var selectionFile: URL { root.appending(path: "selection.json", directoryHint: .notDirectory) }
    private var windowsFile: URL { root.appending(path: "windows.json", directoryHint: .notDirectory) }
    private var tuningFile: URL { root.appending(path: "tuning.json", directoryHint: .notDirectory) }

    private func folder(_ id: ModelID) -> URL {
        root.appending(path: id.directoryName, directoryHint: .isDirectory)
    }

    /// Creates `directory` and `root`, keeping `root` out of backups and
    /// readable after the first unlock, so a download can go on while the
    /// device is locked.
    private func prepare(_ directory: URL) throws {
        let manager = FileManager.default
        if !manager.fileExists(atPath: root.path(percentEncoded: false)) {
            #if os(iOS)
                try manager.createDirectory(
                    at: root, withIntermediateDirectories: true,
                    attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
            #else
                try manager.createDirectory(at: root, withIntermediateDirectories: true)
            #endif
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            var excluded = root
            try excluded.setResourceValues(values)
        }
        try manager.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    private func storage(_ operation: String, _ url: URL, _ body: () throws -> Void) throws(ModelDownloadError) {
        do {
            try body()
        } catch {
            throw .storage(.fileSystem(operation: operation, path: url.path(), reason: "\(error)"))
        }
    }

    private static func partial(_ url: URL) -> URL {
        url.deletingLastPathComponent().appending(path: url.lastPathComponent + ".partial")
    }
}

/// What's written in a model's staging folder while it downloads.
struct Pending: Codable {
    static let name = "pending.json"

    var spec: ModelSpec
    /// Whether the person paused it, or it failed; otherwise the app quit
    /// while it ran, and starts it again.
    var stopped: Bool
}

/// What's written next to a model's files once they're all checked.
private struct Manifest: Codable {
    static let name = "manifest.json"

    var spec: ModelSpec
    var installedAt: Date
}
