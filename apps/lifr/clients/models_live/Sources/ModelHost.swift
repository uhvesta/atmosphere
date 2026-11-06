import ComposableArchitecture
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
/// and unloads it when the system runs low on memory, after it's been idle
/// a while, and before its files are deleted. A turn it stops that way
/// fails as interrupted.
///
/// On iOS an app can't use the GPU in the background. When Lifr goes there
/// mid-turn, the turn stops and starts again where it was: on the CPU at
/// once if the person lets the model answer in the background, or else on
/// the GPU once Lifr is back on screen. `run`'s body is called again for
/// that, and keeps what it had done.
package actor ModelHost {
    package typealias Loader =
        @Sendable (InstalledModel, ModelBackend) async throws(ModelRuntimeError) -> any LiteRTEngine

    private struct Loaded {
        var id: ModelID
        var backend: ModelBackend
        var engine: any LiteRTEngine
    }

    private struct Loading {
        var id: ModelID
        var backend: ModelBackend
        var task: Task<Result<any LiteRTEngine, ModelRuntimeError>, Never>
    }

    private let load: Loader
    private let memory: @Sendable () -> Int64
    private let idle: Duration
    private let sleep: @Sendable (Duration) async throws -> Void
    private let answersInBackground: @Sendable () -> Bool

    private var loaded: Loaded?
    private var loading: Loading?
    private var turns: [UUID: Task<Result<JSONValue, ModelRuntimeError>, Never>] = [:]
    private var interrupted: [UUID: ModelInterruption] = [:]
    private var idleTimer: Task<Void, Never>?
    private var observers: [any NSObjectProtocol] = []
    private var pressure: (any DispatchSourceMemoryPressure)?
    private var inBackground = false
    /// Turns waiting for Lifr to be back on screen.
    private var waiting: [UUID: CheckedContinuation<Void, Never>] = [:]

    /// - Parameters:
    ///   - load: Loads a model's files to run on a backend.
    ///   - memory: The bytes the system leaves Lifr for a model now.
    ///   - idle: How long a model stays loaded without a turn.
    ///   - answersInBackground: Whether a turn goes on, on the CPU, while
    ///     Lifr is in the background.
    package init(
        load: @escaping Loader, memory: @escaping @Sendable () -> Int64 = MemoryPlan.available,
        idle: Duration = .seconds(300),
        sleep: @escaping @Sendable (Duration) async throws -> Void = { try await Task.sleep(for: $0) },
        answersInBackground: @escaping @Sendable () -> Bool = { Shared(.answersInBackground).wrappedValue }
    ) {
        self.load = load
        self.memory = memory
        self.idle = idle
        self.sleep = sleep
        self.answersInBackground = answersInBackground
    }

    /// The model that's loaded, if any.
    package var loadedModel: ModelID? { loaded?.id }

    /// Where the loaded model runs, if one is.
    package var loadedBackend: ModelBackend? { loaded?.backend }

    /// Runs `body` on `model`, loading it first if it isn't. When Lifr goes
    /// to the background mid-turn, `body` runs again where it may: it must
    /// carry on from what it had done.
    package nonisolated func run(
        _ model: InstalledModel,
        _ body: @escaping @Sendable (any LiteRTEngine) async throws(ModelRuntimeError) -> JSONValue
    ) async throws(ModelRuntimeError) -> JSONValue {
        while true {
            await untilAllowed()
            guard !Task.isCancelled else { throw .cancelled }
            let engine: any LiteRTEngine
            do {
                engine = try await self.engine(for: model)
            } catch .interrupted(.background) {
                continue
            }
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
            switch await end(id) {
            case .background?: continue
            case .memory?: throw .interrupted(.memory)
            case nil: return try result.get()
            }
        }
    }

    /// Loads `model` where it can run now, unloading any other, unless it's
    /// loaded already.
    package func engine(for model: InstalledModel) async throws(ModelRuntimeError) -> any LiteRTEngine {
        idleTimer?.cancel()
        let backend: ModelBackend = inBackground ? .cpu : .gpu
        if let loaded, loaded.id == model.id, loaded.backend == backend || !turns.isEmpty { return loaded.engine }
        if let loading, loading.id == model.id, loading.backend == backend { return try await loading.task.value.get() }
        unload(nil)
        let fitting = try await self.backend(for: model, preferring: backend)
        // Another turn may have started loading it while this one waited.
        if let loading, loading.id == model.id { return try await loading.task.value.get() }
        if let loaded, loaded.id == model.id { return loaded.engine }
        if loaded != nil || loading != nil { unload(nil) }
        return try await start(model, on: fitting)
    }

    /// Where `model` fits in memory now: on `preferred`, or on the CPU when
    /// only that fits. The memory of a model unloaded just now takes a
    /// moment to come back, while its last turns let go of it, so it looks
    /// again a few times before it gives up.
    private func backend(
        for model: InstalledModel, preferring preferred: ModelBackend
    ) async throws(ModelRuntimeError) -> ModelBackend {
        let needed = MemoryPlan.needed(model.spec, on: preferred)
        var available = memory()
        for _ in 0..<MemoryPlan.looks where !MemoryPlan.fits(needed, available: available) {
            do {
                try await sleep(MemoryPlan.between)
            } catch {
                throw .cancelled
            }
            available = memory()
        }
        if MemoryPlan.fits(needed, available: available) { return preferred }
        if preferred == .gpu, MemoryPlan.fits(MemoryPlan.needed(model.spec, on: .cpu), available: available) {
            return .cpu
        }
        throw .unavailable(
            .notEnoughMemory(needed: needed + MemoryPlan.headroom(available), available: available))
    }

    /// Loads `model` on `backend`, as the one loaded once it's done.
    private func start(_ model: InstalledModel, on backend: ModelBackend) async throws(ModelRuntimeError)
        -> any LiteRTEngine
    {
        let load = load
        let task = Task { () async -> Result<any LiteRTEngine, ModelRuntimeError> in
            do throws(ModelRuntimeError) {
                return .success(try await load(model, backend))
            } catch {
                return .failure(error)
            }
        }
        loading = Loading(id: model.id, backend: backend, task: task)
        let result = await task.value
        // Unloaded while it loaded: going to the background, or for
        // another model.
        guard let current = loading, current.task == task else {
            throw inBackground ? .interrupted(.background) : .cancelled
        }
        loading = nil
        let engine = try result.get()
        loaded = Loaded(id: model.id, backend: backend, engine: engine)
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

    /// Lifr went to the background, where it can't use the GPU. A model on
    /// the GPU is unloaded, and its turns go on as `run` says.
    package func backgrounded() {
        inBackground = true
        guard loaded?.backend != .cpu, loading?.backend != .cpu else { return }
        unload(turns.isEmpty ? nil : .background)
    }

    /// Lifr is back on screen: turns waiting for it go on. A model on the
    /// CPU stays for the turns it's answering, then makes way for the GPU.
    package func foregrounded() {
        inBackground = false
        for continuation in waiting.values { continuation.resume() }
        waiting = [:]
        if turns.isEmpty, loaded?.backend == .cpu { unload(nil) }
    }

    /// Unloads the model when the system runs low on memory, and on iOS
    /// moves it off the GPU when Lifr goes to the background.
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
                    Task { await self?.backgrounded() }
                },
                center.addObserver(
                    forName: UIApplication.willEnterForegroundNotification, object: nil, queue: nil
                ) { [weak self] _ in
                    Task { await self?.foregrounded() }
                },
            ]
        #endif
    }

    /// Returns once a turn may run: at once on screen, or in the background
    /// when the person lets the model answer there.
    private func untilAllowed() async {
        guard inBackground, !answersInBackground() else { return }
        let id = UUID()
        await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                if Task.isCancelled || !inBackground {
                    continuation.resume()
                } else {
                    waiting[id] = continuation
                }
            }
        } onCancel: {
            Task { await self.stopWaiting(id) }
        }
    }

    private func stopWaiting(_ id: UUID) {
        waiting.removeValue(forKey: id)?.resume()
    }

    private func begin(_ id: UUID, _ task: Task<Result<JSONValue, ModelRuntimeError>, Never>) {
        idleTimer?.cancel()
        turns[id] = task
    }

    private func end(_ id: UUID) -> ModelInterruption? {
        turns[id] = nil
        // In the background, or on the CPU on screen, the model goes once
        // its last turn ends: the memory, or the GPU, is better used.
        if turns.isEmpty, inBackground || loaded?.backend == .cpu { unload(nil) }
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
    /// The memory a model takes once loaded on `backend`: its catalog
    /// figure, or for a pasted model, the size of its files.
    package static func needed(_ spec: ModelSpec, on backend: ModelBackend = .gpu) -> Int64 {
        let gpu = spec.memoryBytes > 0 ? spec.memoryBytes : spec.downloadSize
        return backend == .cpu && spec.cpuMemoryBytes > 0 ? spec.cpuMemoryBytes : gpu
    }

    /// How many more times a model that doesn't fit looks again, and how
    /// long it waits before each: memory given back takes a moment.
    static let looks = 6
    static let between: Duration = .milliseconds(500)

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
