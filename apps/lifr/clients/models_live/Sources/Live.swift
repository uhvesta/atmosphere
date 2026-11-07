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
            selection: { [selection] use in selection.model(for: use) },
            select: { [self] id, use in await select(id, for: use) },
            setWindow: { [self] id, window in await setWindow(window, for: id) }
        )
    }
}
