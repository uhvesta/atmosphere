import Foundation
import InferenceKit
import JSONKit

/// A downloaded model, run through `host` by the engine that loads it: on
/// the GPU, or on the CPU while it answers in the background.
public struct EngineRuntime: ModelRuntime {
    public let model: InstalledModel
    public let host: ModelHost

    public init(model: InstalledModel, host: ModelHost) {
        self.model = model
        self.host = host
    }

    /// What's kept for the answer and the tools' results: the same share
    /// of the window as Apple's model keeps, and no less.
    public var budget: ContextBudget {
        let window = model.window
        return ContextBudget(
            window: window,
            reservedForAnswer: max(ContextBudget.appleOnDevice.reservedForAnswer, window * 1_100 / 4_096))
    }

    public var inputs: Set<ModelInput> { model.spec.inputs }

    public var status: ModelStatus {
        let files = model.spec.source?.files ?? []
        let present = files.allSatisfy { FileManager.default.fileExists(atPath: model.location(of: $0).path) }
        return !files.isEmpty && present ? .available : .notDownloaded
    }

    public func prewarm() {
        let model = model
        let host = host
        Task { _ = try? await host.engine(for: model) }
    }

    public func respond(to turn: ModelTurn, partial: @escaping @Sendable (JSONValue) -> Void)
        async throws(ModelRuntimeError) -> JSONValue
    {
        let status = status
        guard status == .available else { throw .unavailable(status) }
        if let kind = turn.unreadable(by: inputs) { throw .unsupportedInput(kind) }
        // Kept across the host's restarts of the turn, so it carries on.
        let journal = EngineTurn.Journal()
        return try await host.run(model) { engine async throws(ModelRuntimeError) in
            try await EngineTurn.respond(to: turn, on: engine, journal: journal, partial: partial)
        }
    }
}

/// Picks the runtime for each use: the model the person picked for it if
/// it's downloaded, or Apple's. Changes to the library go through here, so
/// a model is unloaded before it's deleted, and memory isn't held for one
/// no use runs on.
public struct ModelRuntimes<Use: ModelUseCase>: Sendable {
    public let library: ModelLibrary
    public let selection: ModelSelection<Use>
    public let host: ModelHost

    /// - Parameter selection: What each use runs on; by default kept in
    ///   `selection.json` in the library's folder.
    public init(library: ModelLibrary, selection: ModelSelection<Use>? = nil, host: ModelHost) {
        self.library = library
        self.selection = selection ?? ModelSelection(file: library.selectionFile)
        self.host = host
    }

    public func runtime(for use: Use) -> any ModelRuntime {
        let id = selection.model(for: use)
        guard id != .apple else { return AppleRuntime() }
        guard let model = library.installed().first(where: { $0.id == id }) else { return Missing() }
        switch model.spec.runtime {
        case .apple: return AppleRuntime()
        case .liteRT: return EngineRuntime(model: model, host: host)
        }
    }

    /// Unloads the model, removes its files, and sends the uses that ran on
    /// it back to their defaults.
    public func delete(_ id: ModelID) async throws(ModelStorageError) {
        await host.unload(model: id)
        try await library.delete(id)
        selection.forget(id)
    }

    /// Runs `id` at `window`, and lets go of the memory held for the old one.
    public func setWindow(_ window: Int, for id: ModelID) async {
        library.setWindow(window, for: id)
        await host.release(model: id)
    }

    /// Runs `use` on `id`, and lets go of the loaded model if no use runs
    /// on it now.
    public func select(_ id: ModelID, for use: Use) async {
        selection.select(id, for: use)
        guard let loaded = await host.loadedModel, !selection.isUsed(loaded) else { return }
        await host.release(model: loaded)
    }

    /// A model that was picked and isn't on the device.
    private struct Missing: ModelRuntime {
        var status: ModelStatus { .notDownloaded }
        var budget: ContextBudget { .appleOnDevice }
        var inputs: Set<ModelInput> { [] }
        func prewarm() {}
        func respond(to turn: ModelTurn, partial: @escaping @Sendable (JSONValue) -> Void)
            async throws(ModelRuntimeError) -> JSONValue
        {
            throw .unavailable(.notDownloaded)
        }
    }
}
