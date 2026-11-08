import ComposableArchitecture
@_exported import InferenceKitLive
import LifrModelsClient

extension ModelLibrary {
    /// Lifr's library, in Application Support.
    package static func live() -> ModelLibrary { live(userAgent: "Lifr") }
}

extension ModelRuntimes where Use == ModelUse {
    /// The library and the loaded model, for features.
    package var client: ModelLibraryClient {
        ModelLibraryClient(
            catalog: { [library] in library.catalog },
            installed: { [library] in library.installed() },
            unfinished: { [library] in library.unfinishedModels() },
            resolve: { [library] link, sha256 throws(ModelResolveError) in
                try await library.resolve(link, sha256: sha256)
            },
            download: { [library] spec throws(ModelDownloadError) in try await library.download(spec) },
            downloads: { [library] in library.downloads() },
            pause: { [library] id in await library.pause(id) },
            delete: { [self] id throws(ModelStorageError) in try await delete(id) },
            tune: { [self] id throws(ModelRuntimeError) in _ = try await tune(id) },
            pickBackend: { [self] id, backend in await pick(backend, for: id) },
            tuning: { [self] id in tuning(for: id) },
            tuneCandidates: { [self] id in tuneCandidates(for: id) },
            compare: { [self] candidates, report in await tune(candidates, report: report) },
            tuneResults: { [self] id in tuneResults(for: id) },
            setups: { [self] id in setups(for: id) },
            pickSetup: { [self] setup, purpose, id in await pick(setup, for: purpose, of: id) },
            otherFormats: { [self] id in otherFormats(of: id) },
            uses: { ModelUse.allCases.map(ModelUseKey.init) },
            selection: { [selection] key in ModelUse(rawValue: key.rawValue).map(selection.model(for:)) ?? .apple },
            select: { [self] id, key in
                guard let use = ModelUse(rawValue: key.rawValue) else { return }
                await select(id, for: use)
            },
            setWindow: { [self] id, window in await setWindow(window, for: id) }
        )
    }
}
