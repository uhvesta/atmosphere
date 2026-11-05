import Foundation
import LifrAPI
import LifrModelsClient

/// A downloaded model, run by LiteRT-LM on the GPU through `host`.
package struct LiteRTRuntime: ModelRuntime {
    package let model: InstalledModel
    package let host: ModelHost

    package init(model: InstalledModel, host: ModelHost) {
        self.model = model
        self.host = host
    }

    /// What's kept for the answer and the tools' results: the same share
    /// of the window as Apple's model keeps, and no less.
    package var budget: ContextBudget {
        let window = model.spec.contextTokens
        return ContextBudget(
            window: window,
            reservedForAnswer: max(ContextBudget.appleOnDevice.reservedForAnswer, window * 1_100 / 4_096))
    }

    package var status: ModelStatus {
        let files = model.spec.source?.files ?? []
        let present = files.allSatisfy { FileManager.default.fileExists(atPath: model.location(of: $0).path) }
        return !files.isEmpty && present ? .available : .notDownloaded
    }

    package func prewarm() {
        let model = model
        let host = host
        Task { _ = try? await host.engine(for: model) }
    }

    package func respond(to turn: ModelTurn, partial: @escaping @Sendable (JSONValue) -> Void)
        async throws(ModelRuntimeError) -> JSONValue
    {
        let status = status
        guard status == .available else { throw .unavailable(status) }
        return try await host.run(model) { engine async throws(ModelRuntimeError) in
            try await LiteRTTurn.respond(to: turn, on: engine, partial: partial)
        }
    }
}

/// Picks the runtime for each use: the model the person picked for it if
/// it's downloaded, or Apple's.
package struct ModelRuntimes: Sendable {
    package let library: ModelLibrary
    package let host: ModelHost

    package init(library: ModelLibrary, host: ModelHost) {
        self.library = library
        self.host = host
    }

    package func runtime(for use: ModelUse) -> any ModelRuntime {
        let id = library.selection(for: use)
        guard id != .apple else { return AppleRuntime() }
        guard let model = library.installed().first(where: { $0.id == id }) else { return Missing() }
        switch model.spec.runtime {
        case .apple: return AppleRuntime()
        case .liteRT: return LiteRTRuntime(model: model, host: host)
        }
    }

    /// The library, with each model unloaded before it's deleted.
    package var client: ModelLibraryClient {
        var client = library.client
        let host = host
        let delete = client.delete
        client.delete = { id throws(ModelStorageError) in
            await host.unload(model: id)
            try await delete(id)
        }
        return client
    }

    /// A model that was picked and isn't on the device.
    private struct Missing: ModelRuntime {
        var status: ModelStatus { .notDownloaded }
        var budget: ContextBudget { .appleOnDevice }
        func prewarm() {}
        func respond(to turn: ModelTurn, partial: @escaping @Sendable (JSONValue) -> Void)
            async throws(ModelRuntimeError) -> JSONValue
        {
            throw .unavailable(.notDownloaded)
        }
    }
}
