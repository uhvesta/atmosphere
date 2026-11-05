import Foundation
import LifrAPI
import LifrModelsClient

#if canImport(UIKit)
    import UIKit
#endif

#if os(iOS)
    import os
#else
    import Metal
#endif

/// Keeps at most one downloaded model loaded, since each takes gigabytes.
/// It loads a model for its first turn, after checking it fits in memory,
/// and unloads it when the system runs low on memory, when Lifr goes to
/// the background on iOS, after it's been idle a while, and before its
/// files are deleted. A turn it stops that way fails as interrupted.
package actor ModelHost {
    package typealias Loader = @Sendable (InstalledModel) async throws(ModelRuntimeError) -> any LiteRTEngine

    private let load: Loader
    private let memory: @Sendable () -> Int64
    private let idle: Duration
    private let sleep: @Sendable (Duration) async throws -> Void

    private var loaded: (id: ModelID, engine: any LiteRTEngine)?
    private var loading: (id: ModelID, task: Task<Result<any LiteRTEngine, ModelRuntimeError>, Never>)?
    private var turns: [UUID: Task<Result<JSONValue, ModelRuntimeError>, Never>] = [:]
    private var interrupted: [UUID: ModelInterruption] = [:]
    private var idleTimer: Task<Void, Never>?
    private var observers: [any NSObjectProtocol] = []
    private var pressure: (any DispatchSourceMemoryPressure)?

    /// - Parameters:
    ///   - load: Loads a model's files.
    ///   - memory: The bytes the system leaves Lifr for a model now.
    ///   - idle: How long a model stays loaded without a turn.
    package init(
        load: @escaping Loader, memory: @escaping @Sendable () -> Int64 = MemoryPlan.available,
        idle: Duration = .seconds(300),
        sleep: @escaping @Sendable (Duration) async throws -> Void = { try await Task.sleep(for: $0) }
    ) {
        self.load = load
        self.memory = memory
        self.idle = idle
        self.sleep = sleep
    }

    /// The model that's loaded, if any.
    package var loadedModel: ModelID? { loaded?.id }

    /// Runs `body` on `model`, loading it first if it isn't.
    package nonisolated func run(
        _ model: InstalledModel,
        _ body: @escaping @Sendable (any LiteRTEngine) async throws(ModelRuntimeError) -> JSONValue
    ) async throws(ModelRuntimeError) -> JSONValue {
        let engine = try await engine(for: model)
        let id = UUID()
        let task = Task { () async -> Result<JSONValue, ModelRuntimeError> in
            do throws(ModelRuntimeError) {
                return .success(try await body(engine))
            } catch {
                return .failure(error)
            }
        }
        await begin(id, task)
        let result = await withTaskCancellationHandler {
            await task.value
        } onCancel: {
            task.cancel()
        }
        if let reason = await end(id) { throw .interrupted(reason) }
        return try result.get()
    }

    /// Loads `model`, unloading any other, unless it's loaded already.
    package func engine(for model: InstalledModel) async throws(ModelRuntimeError) -> any LiteRTEngine {
        idleTimer?.cancel()
        if let loaded, loaded.id == model.id { return loaded.engine }
        if let loading, loading.id == model.id { return try await loading.task.value.get() }
        unload(nil)
        let needed = MemoryPlan.needed(model.spec)
        let available = memory()
        guard MemoryPlan.fits(needed, available: available) else {
            throw .unavailable(.notEnoughMemory(needed: needed, available: available))
        }
        let load = load
        let task = Task { () async -> Result<any LiteRTEngine, ModelRuntimeError> in
            do throws(ModelRuntimeError) {
                return .success(try await load(model))
            } catch {
                return .failure(error)
            }
        }
        loading = (model.id, task)
        let result = await task.value
        if loading?.id == model.id { loading = nil }
        let engine = try result.get()
        loaded = (model.id, engine)
        scheduleIdle()
        return engine
    }

    /// Unloads the model, stopping its turns. Each fails as interrupted for
    /// `reason`, or as cancelled without one.
    package func unload(_ reason: ModelInterruption?) {
        for (id, task) in turns {
            if let reason { interrupted[id] = reason }
            task.cancel()
        }
        loaded = nil
        loading?.task.cancel()
        loading = nil
        idleTimer?.cancel()
        idleTimer = nil
    }

    /// Unloads `id` if it's loaded, as before its files are deleted.
    package func unload(model id: ModelID) {
        if loaded?.id == id || loading?.id == id { unload(nil) }
    }

    /// Unloads the model when the system runs low on memory, and on iOS
    /// when Lifr goes to the background.
    package func observeSystem() {
        guard pressure == nil else { return }
        let source = DispatchSource.makeMemoryPressureSource(eventMask: [.warning, .critical], queue: .global())
        source.setEventHandler { [weak self] in
            Task { await self?.unload(.memory) }
        }
        source.resume()
        pressure = source
        #if canImport(UIKit) && !os(watchOS)
            let center = NotificationCenter.default
            observers = [
                center.addObserver(
                    forName: UIApplication.didReceiveMemoryWarningNotification, object: nil, queue: nil
                ) { [weak self] _ in
                    Task { await self?.unload(.memory) }
                },
                center.addObserver(
                    forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: nil
                ) { [weak self] _ in
                    Task { await self?.unload(.background) }
                },
            ]
        #endif
    }

    private func begin(_ id: UUID, _ task: Task<Result<JSONValue, ModelRuntimeError>, Never>) {
        idleTimer?.cancel()
        turns[id] = task
    }

    private func end(_ id: UUID) -> ModelInterruption? {
        turns[id] = nil
        scheduleIdle()
        return interrupted.removeValue(forKey: id)
    }

    private func scheduleIdle() {
        guard turns.isEmpty, loaded != nil else { return }
        idleTimer?.cancel()
        let idle = idle
        let sleep = sleep
        idleTimer = Task { [weak self] in
            guard (try? await sleep(idle)) != nil, !Task.isCancelled else { return }
            await self?.unloadIfIdle()
        }
    }

    private func unloadIfIdle() {
        if turns.isEmpty { unload(nil) }
    }
}

/// Whether a model fits in the memory the system leaves Lifr.
package enum MemoryPlan {
    /// The memory a model takes once loaded: its catalog figure, or for a
    /// pasted model, the size of its files.
    package static func needed(_ spec: ModelSpec) -> Int64 {
        spec.memoryBytes > 0 ? spec.memoryBytes : spec.downloadSize
    }

    /// What's kept free besides the model: half a gigabyte, or a tenth of
    /// what's available if that's more.
    package static func headroom(_ available: Int64) -> Int64 {
        max(500_000_000, available / 10)
    }

    package static func fits(_ needed: Int64, available: Int64) -> Bool {
        needed + headroom(available) <= available
    }

    /// The bytes the system leaves Lifr now. On iOS that's what the app may
    /// still allocate; on the Mac, the smaller of 60% of the memory and what
    /// Metal recommends the GPU uses.
    @Sendable package static func available() -> Int64 {
        #if os(iOS)
            Int64(os_proc_available_memory())
        #else
            let physical = Int64(clamping: ProcessInfo.processInfo.physicalMemory)
            let share = physical / 10 * 6
            guard let gpu = MTLCreateSystemDefaultDevice()?.recommendedMaxWorkingSetSize else { return share }
            return min(share, Int64(clamping: gpu))
        #endif
    }
}
