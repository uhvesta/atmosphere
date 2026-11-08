import Foundation
import InferenceKit
import JSONKit

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
/// On iOS an app can't use the GPU in the background. When the app goes there
/// mid-turn, the turn stops and starts again where it was: on the CPU at
/// once if the person lets the model answer in the background, or else on
/// the GPU once the app is back on screen. `run`'s body is called again for
/// that, and keeps what it had done.
public actor ModelHost {
    public typealias Loader =
        @Sendable (InstalledModel, ModelBackend) async throws(ModelRuntimeError) -> any ModelEngine

    private struct Loaded {
        var id: ModelID
        var window: Int
        var backend: ModelBackend
        var engine: any ModelEngine

        func `is`(_ model: InstalledModel) -> Bool { id == model.id && window == model.window }
    }

    private struct Loading {
        var id: ModelID
        var window: Int
        var backend: ModelBackend
        var task: Task<Result<any ModelEngine, ModelRuntimeError>, Never>

        func `is`(_ model: InstalledModel) -> Bool { id == model.id && window == model.window }
    }

    private let load: Loader
    private let memory: @Sendable () -> Int64
    private let idle: Duration
    private let sleep: @Sendable (Duration) async throws -> Void
    private let answersInBackground: @Sendable () -> Bool
    private let device: DeviceInfo

    private var loaded: Loaded?
    private var loading: Loading?
    private var turns: [UUID: Task<Result<JSONValue, ModelRuntimeError>, Never>] = [:]
    private var interrupted: [UUID: ModelInterruption] = [:]
    private var idleTimer: Task<Void, Never>?
    private var observers: [any NSObjectProtocol] = []
    private var pressure: (any DispatchSourceMemoryPressure)?
    private var inBackground = false
    /// Turns waiting for the app to be back on screen.
    private var waiting: [UUID: CheckedContinuation<Void, Never>] = [:]

    /// - Parameters:
    ///   - load: Loads a model's files to run on a backend.
    ///   - memory: The bytes the system leaves the app for a model now.
    ///   - idle: How long a model stays loaded without a turn.
    ///   - answersInBackground: Whether a turn goes on, on the CPU, while
    ///     the app is in the background. Off, it waits for the app to be
    ///     back on screen.
    ///   - device: The device measurements must be from to count.
    public init(
        load: @escaping Loader, memory: @escaping @Sendable () -> Int64 = MemoryPlan.available,
        idle: Duration = .seconds(300),
        sleep: @escaping @Sendable (Duration) async throws -> Void = { try await Task.sleep(for: $0) },
        answersInBackground: @escaping @Sendable () -> Bool = { false }, device: DeviceInfo = .current
    ) {
        self.device = device
        self.load = load
        self.memory = memory
        self.idle = idle
        self.sleep = sleep
        self.answersInBackground = answersInBackground
    }

    /// Loads each model with its own engine, of `engines`: those the app is
    /// built with. A model whose engine isn't among them fails to load,
    /// saying so.
    public static func loading(with engines: [RuntimeKind: Loader]) -> Loader {
        { model, backend async throws(ModelRuntimeError) in
            let kind = model.spec.runtime
            guard let load = engines[kind] else { throw .engineMissing(kind) }
            return try await load(model, backend)
        }
    }

    /// Where `model` runs while the app is on screen, and why: the person's
    /// pick, what was measured fastest here among what fits in memory now,
    /// or its profile's.
    public nonisolated func plan(for model: InstalledModel) -> ModelPlanner.Plan {
        let available = memory()
        return ModelPlanner.plan(
            model, picked: model.pickedBackend, measurements: model.measurements, device: device.model,
            system: device.system, fits: { MemoryPlan.fits($0, available: available) })
    }

    /// Where `model` runs and why, the person's pick, and what was measured
    /// of it on this device at its window.
    public nonisolated func tuning(for model: InstalledModel) -> ModelTuning {
        let here = model.measurements
            .filter { $0.describes(model, device: device.model, system: device.system) }
            .sorted { $0.measuredAt > $1.measuredAt }
        var latest: [ModelMeasurement] = []
        for measurement in here where !latest.contains(where: { $0.backend == measurement.backend }) {
            latest.append(measurement)
        }
        return ModelTuning(
            plan: plan(for: model), picked: model.pickedBackend, measurements: latest,
            backends: model.spec.runtime.backends)
    }

    /// Whether `model` fits on `backend` in the memory the system leaves
    /// the app now, with nothing else loaded.
    public nonisolated func fits(_ model: InstalledModel, on backend: ModelBackend) -> Bool {
        MemoryPlan.fits(MemoryPlan.needed(model, on: backend), available: memory())
    }

    /// The model that's loaded, if any.
    public var loadedModel: ModelID? { loaded?.id }

    /// Where the loaded model runs, if one is.
    public var loadedBackend: ModelBackend? { loaded?.backend }

    /// Runs `body` on `model`, loading it first if it isn't. When the app goes
    /// to the background mid-turn, `body` runs again where it may: it must
    /// carry on from what it had done.
    public nonisolated func run(
        _ model: InstalledModel,
        _ body: @escaping @Sendable (any ModelEngine) async throws(ModelRuntimeError) -> JSONValue
    ) async throws(ModelRuntimeError) -> JSONValue {
        while true {
            await untilAllowed(model)
            guard !Task.isCancelled else { throw .cancelled }
            let engine: any ModelEngine
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
    public func engine(for model: InstalledModel) async throws(ModelRuntimeError) -> any ModelEngine {
        idleTimer?.cancel()
        // In the background only the CPU is left, which not every runtime
        // runs on: its turns wait for the screen.
        if inBackground, !model.spec.runtime.backends.contains(.cpu) { throw .interrupted(.background) }
        let backend: ModelBackend = inBackground ? .cpu : plan(for: model).backend
        // A model loaded at another window is loaded again, once its turns
        // are done.
        if let loaded, loaded.is(model), loaded.backend == backend || !turns.isEmpty { return loaded.engine }
        if let loading, loading.is(model), loading.backend == backend { return try await loading.task.value.get() }
        unload(nil)
        let fitting = try await self.backend(for: model, preferring: backend)
        // Another turn may have started loading it while this one waited.
        if let loading, loading.is(model) { return try await loading.task.value.get() }
        if let loaded, loaded.is(model) { return loaded.engine }
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
        let needed = MemoryPlan.needed(model, on: preferred)
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
        if preferred == .gpu, model.spec.runtime.backends.contains(.cpu),
            MemoryPlan.fits(MemoryPlan.needed(model, on: .cpu), available: available)
        {
            return .cpu
        }
        throw .unavailable(
            .notEnoughMemory(needed: needed + MemoryPlan.headroom(available), available: available))
    }

    /// Loads `model` on `backend`, as the one loaded once it's done.
    private func start(_ model: InstalledModel, on backend: ModelBackend) async throws(ModelRuntimeError)
        -> any ModelEngine
    {
        let load = load
        let task = Task { () async -> Result<any ModelEngine, ModelRuntimeError> in
            do throws(ModelRuntimeError) {
                return .success(try await load(model, backend))
            } catch {
                return .failure(error)
            }
        }
        loading = Loading(id: model.id, window: model.window, backend: backend, task: task)
        let result = await task.value
        // Unloaded while it loaded: going to the background, or for
        // another model.
        guard let current = loading, current.task == task else {
            throw inBackground ? .interrupted(.background) : .cancelled
        }
        loading = nil
        let engine = try result.get()
        loaded = Loaded(id: model.id, window: model.window, backend: backend, engine: engine)
        scheduleIdle()
        return engine
    }

    /// Unloads the model, stopping its turns. Each fails as interrupted for
    /// `reason`, or as cancelled without one.
    public func unload(_ reason: ModelInterruption?) {
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

    /// Unloads `id` if it's loaded and answering nothing, as when the person
    /// picks another window for it or stops using it. A turn running on it
    /// finishes first; the model then goes when it's idle.
    public func release(model id: ModelID) {
        guard turns.isEmpty, loaded?.id == id || loading?.id == id else { return }
        unload(nil)
    }

    /// Unloads `id` if it's loaded, as before its files are deleted.
    public func unload(model id: ModelID) {
        if loaded?.id == id || loading?.id == id { unload(nil) }
    }

    /// The app went to the background, where it can't use the GPU. A model on
    /// the GPU is unloaded, and its turns go on as `run` says.
    public func backgrounded() {
        inBackground = true
        guard loaded?.backend != .cpu, loading?.backend != .cpu else { return }
        unload(turns.isEmpty ? nil : .background)
    }

    /// The app is back on screen: turns waiting for it go on. A model on the
    /// CPU stays for the turns it's answering, then makes way for the GPU.
    public func foregrounded() {
        inBackground = false
        for continuation in waiting.values { continuation.resume() }
        waiting = [:]
        if turns.isEmpty, loaded?.backend == .cpu { unload(nil) }
    }

    /// Unloads the model when the system runs low on memory, and on iOS
    /// moves it off the GPU when the app goes to the background.
    public func observeSystem() {
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

    /// Returns once a turn on `model` may run: at once on screen, or in the
    /// background when the person lets models answer there and its runtime
    /// runs on the CPU.
    private func untilAllowed(_ model: InstalledModel) async {
        guard inBackground, !answersInBackground() || !model.spec.runtime.backends.contains(.cpu) else { return }
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

/// Whether a model fits in the memory the system leaves the app.
public enum MemoryPlan {
    /// The memory a model takes once loaded on `backend` at its window: its
    /// catalog figure, or for a pasted model, the size of its files. The
    /// window's share is the same on the CPU as on the GPU.
    public static func needed(_ model: InstalledModel, on backend: ModelBackend = .gpu) -> Int64 {
        let spec = model.spec
        let gpu = spec.memoryBytes > 0 ? spec.memoryBytes(at: model.window) : spec.downloadSize
        guard backend == .cpu, spec.cpuMemoryBytes > 0 else { return gpu }
        return spec.cpuMemoryBytes + max(0, gpu - spec.memoryBytes)
    }

    /// How many more times a model that doesn't fit looks again, and how
    /// long it waits before each: memory given back takes a moment.
    static let looks = 6
    static let between: Duration = .milliseconds(500)

    /// What's kept free besides the model: half a gigabyte, or a tenth of
    /// what's available if that's more.
    public static func headroom(_ available: Int64) -> Int64 {
        max(500_000_000, available / 10)
    }

    public static func fits(_ needed: Int64, available: Int64) -> Bool {
        needed + headroom(available) <= available
    }

    /// The bytes the system leaves the app now. On iOS that's what the app may
    /// still allocate; on the Mac, the smaller of 60% of the memory and what
    /// Metal recommends the GPU uses.
    @Sendable public static func available() -> Int64 {
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
