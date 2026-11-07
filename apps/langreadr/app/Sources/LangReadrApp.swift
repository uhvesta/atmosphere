import ComposableArchitecture
import Foundation
import InferenceKit
import InferenceKitLiteRT
import InferenceKitLive
import LangReadrCore
import LangReadrModels
import LangReadrReading
import SwiftUI

#if canImport(InferenceKitLlama)
    import InferenceKitLlama
#endif

/// LangReadr: read what you import in the language you're learning, with
/// a model on the device to translate it and answer questions about it.
@main
struct LangReadrApp: App {
    @State private var store = Store(initialState: LibraryFeature.State()) { LibraryFeature() }

    var body: some Scene {
        #if os(macOS)
            window
        #else
            window
                // Woken for downloads the system finished while LangReadr
                // was away.
                .backgroundTask(.urlSession(ModelLibrary.backgroundSession)) {
                    _ = Models.runtimes
                    await ModelLibrary.caughtUp()
                }
        #endif
    }

    private var window: some Scene {
        WindowGroup {
            LangReadrView(store: store)
        }
    }
}

private enum Models {
    /// The models downloaded to this device, and the one that's loaded.
    static let runtimes: ModelRuntimes<ModelUse> = {
        var loaders: [RuntimeKind: ModelHost.Loader] = [
            .liteRT: { model, backend async throws(ModelRuntimeError) in
                try await LiteRTLMEngine.load(model, on: backend)
            }
        ]
        var measures: [RuntimeKind: Bench.Measure] = [.liteRT: LiteRTLMEngine.measure]
        // llama.cpp has no build for the Simulator. A closure that throws a
        // type crashes the compiler when it's stored straight into a
        // dictionary's subscript, so each gets its type first.
        #if canImport(InferenceKitLlama)
            let llama: ModelHost.Loader = { model, backend async throws(ModelRuntimeError) in
                try await LlamaEngine.load(model, on: backend)
            }
            let measure: Bench.Measure = LlamaEngine.measure
            loaders[.llama] = llama
            measures[.llama] = measure
        #endif
        // A model stops when LangReadr leaves the screen, and carries on
        // when it's back.
        let host = ModelHost(load: ModelHost.loading(with: loaders), answersInBackground: { false })
        Task { await host.observeSystem() }
        let library = ModelLibrary.live(userAgent: "LangReadr")
        library.resumeUnfinished()
        let bench = Bench(measure: Bench.measuring(with: measures))
        return ModelRuntimes(library: library, host: host, bench: bench)
    }()
}

/// The tutor on the models picked for each use, explaining in the
/// device's language.
extension TutorKey: DependencyKey {
    package static let liveValue = Tutor(
        runtime: { use, choice in Models.runtimes.runtime(for: use, choice: choice) },
        reader: Language(Locale.current.language.languageCode?.identifier ?? "en"))
}

extension ModelLibraryClient: DependencyKey {
    package static let liveValue: ModelLibraryClient = {
        let models = Models.runtimes
        return ModelLibraryClient(
            catalog: { models.library.catalog },
            installed: { models.library.installed() },
            unfinished: { models.library.unfinishedModels() },
            resolve: { link, sha256 throws(ModelResolveError) in try await models.library.resolve(link, sha256: sha256)
            },
            download: { spec throws(ModelDownloadError) in try await models.library.download(spec) },
            downloads: { models.library.downloads() },
            pause: { id in await models.library.pause(id) },
            delete: { id throws(ModelStorageError) in try await models.delete(id) },
            tune: { id throws(ModelRuntimeError) in _ = try await models.tune(id) },
            pickBackend: { id, backend in await models.pick(backend, for: id) },
            tuning: { id in models.tuning(for: id) },
            selection: { use in models.selection.model(for: use) },
            select: { id, use in await models.select(id, for: use) },
            setWindow: { id, window in await models.setWindow(window, for: id) }
        )
    }()
}
