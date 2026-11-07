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
    public let bench: Bench?

    /// - Parameters:
    ///   - selection: What each use runs on; by default kept in
    ///     `selection.json` in the library's folder.
    ///   - bench: What measures a model when the person asks; nil when the
    ///     app can't.
    public init(
        library: ModelLibrary, selection: ModelSelection<Use>? = nil, host: ModelHost, bench: Bench? = nil
    ) {
        self.library = library
        self.selection = selection ?? ModelSelection(file: library.selectionFile)
        self.host = host
        self.bench = bench
    }

    /// What `use` runs on, or what `choice` picks in place of it: its
    /// model, at its window. The person's defaults stay as they are.
    public func runtime(for use: Use, choice: ModelChoice = .defaults) -> any ModelRuntime {
        let id = choice.model ?? selection.model(for: use)
        guard id != .apple else { return AppleRuntime() }
        guard var model = library.installed().first(where: { $0.id == id }) else { return Missing() }
        if let window = choice.window, window == model.spec.contextTokens || model.spec.contextChoices.contains(window)
        {
            model.chosenWindow = window
        }
        switch model.spec.runtime {
        case .apple: return AppleRuntime()
        case .liteRT, .llama: return EngineRuntime(model: model, host: host)
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

    /// Measures `id` on each backend it fits on now, and keeps the
    /// measurements for the planner. What's loaded is unloaded first.
    public func tune(_ id: ModelID) async throws(ModelRuntimeError) -> [ModelMeasurement] {
        guard let bench, let model = library.installed().first(where: { $0.id == id }) else {
            throw .unavailable(.notDownloaded)
        }
        let backends = [ModelBackend.gpu, .cpu].filter { host.fits(model, on: $0) }
        guard !backends.isEmpty else {
            let needed = MemoryPlan.needed(model, on: .cpu)
            let available = MemoryPlan.available()
            throw .unavailable(
                .notEnoughMemory(needed: needed + MemoryPlan.headroom(available), available: available))
        }
        let measured = try await bench.run(model, on: backends, host: host)
        library.record(measured, for: id)
        return measured
    }

    /// Runs `id` on `backend` while the app is on screen, or where the
    /// planner picks with nil, from its next turn.
    public func pick(_ backend: ModelBackend?, for id: ModelID) async {
        library.pick(backend, for: id)
        await host.release(model: id)
    }

    /// Where `id` runs on screen and why, and what was measured of it
    /// here, if it's downloaded.
    public func tuning(for id: ModelID) -> ModelTuning? {
        library.installed().first { $0.id == id }.map(host.tuning)
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
