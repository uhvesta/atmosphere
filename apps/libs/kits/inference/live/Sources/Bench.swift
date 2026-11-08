import Darwin
import Foundation
import InferenceKit

/// Which device and system this is, as measurements record them.
public struct DeviceInfo: Hashable, Sendable {
    /// Such as iPhone18,1 or Mac16,10.
    public var model: String
    /// Such as 27.0.
    public var system: String

    public init(model: String, system: String) {
        self.model = model
        self.system = system
    }

    public static let current: DeviceInfo = {
        #if os(iOS)
            let key = "hw.machine"
        #else
            let key = "hw.model"
        #endif
        var size = 0
        sysctlbyname(key, nil, &size, nil, 0)
        var bytes = [CChar](repeating: 0, count: max(size, 1))
        sysctlbyname(key, &bytes, &size, nil, 0)
        let model =
            String(bytes: bytes.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, encoding: .utf8) ?? "unknown"
        let version = ProcessInfo.processInfo.operatingSystemVersion
        return DeviceInfo(model: model, system: "\(version.majorVersion).\(version.minorVersion)")
    }()
}

/// What the app has drawn so far, as the system counts it against the app.
public struct TaskEnergy: Hashable, Sendable {
    /// The energy the CPU drew for the app, in joules.
    public var cpuJoules: Double
    /// How long the GPU has been busy for the app, in seconds.
    public var gpuSeconds: Double

    public init(cpuJoules: Double, gpuSeconds: Double) {
        self.cpuJoules = cpuJoules
        self.gpuSeconds = gpuSeconds
    }
}

/// Measures a downloaded model on each backend, only when the person asks:
/// a short fixed prompt read and answered, with the memory the app took,
/// the CPU it used and the energy the CPU drew. Nothing else runs on the model meanwhile.
public struct Bench: Sendable {
    /// Runs the short prompt on `model` on a backend, with the engine that
    /// loads it, and says how fast it read and wrote.
    public typealias Measure =
        @Sendable (InstalledModel, ModelBackend) async throws(ModelRuntimeError) -> EngineSpeed

    private let measure: Measure
    /// Loads a model for its answers to the quality prompts.
    let load: ModelHost.Loader?
    let device: DeviceInfo
    private let footprint: @Sendable () -> Int64
    private let cpuSeconds: @Sendable () -> Double
    private let energy: @Sendable () -> TaskEnergy?
    private let now: @Sendable () -> Date
    private let interval: Duration

    /// - Parameters:
    ///   - load: Loads a model to answer the quality prompts when it's
    ///     tuned; nil leaves quality unmeasured.
    ///   - footprint: The memory the app takes now.
    ///   - cpuSeconds: The CPU time the app has used, across its cores.
    ///   - energy: What the app has drawn, or nil where the system doesn't
    ///     say.
    ///   - interval: How often the memory is looked at during a run.
    public init(
        measure: @escaping Measure, load: ModelHost.Loader? = nil, device: DeviceInfo = .current,
        footprint: @escaping @Sendable () -> Int64 = Bench.footprint,
        cpuSeconds: @escaping @Sendable () -> Double = Bench.cpuSeconds,
        energy: @escaping @Sendable () -> TaskEnergy? = Bench.energy,
        now: @escaping @Sendable () -> Date = Date.init, interval: Duration = .milliseconds(20)
    ) {
        self.measure = measure
        self.load = load
        self.device = device
        self.footprint = footprint
        self.cpuSeconds = cpuSeconds
        self.energy = energy
        self.now = now
        self.interval = interval
    }

    /// Measures each model with its own engine, of `engines`.
    public static func measuring(with engines: [RuntimeKind: Measure]) -> Measure {
        { model, backend async throws(ModelRuntimeError) in
            let kind = model.spec.runtime
            guard let measure = engines[kind] else { throw .engineMissing(kind) }
            return try await measure(model, backend)
        }
    }

    /// Measures `model` on each of `backends`, unloading what `host` holds
    /// first. A backend that fails is left out; when every one fails, the
    /// first one's error is thrown.
    public func run(
        _ model: InstalledModel, on backends: [ModelBackend], host: ModelHost
    ) async throws(ModelRuntimeError) -> [ModelMeasurement] {
        await host.unload(nil)
        var measured: [ModelMeasurement] = []
        var firstError: ModelRuntimeError?
        for backend in backends {
            if Task.isCancelled { throw .cancelled }
            do throws(ModelRuntimeError) {
                measured.append(try await once(model, on: backend))
            } catch .cancelled {
                throw .cancelled
            } catch {
                firstError = firstError ?? error
            }
        }
        if measured.isEmpty, let firstError { throw firstError }
        return measured
    }

    func once(_ model: InstalledModel, on backend: ModelBackend) async throws(ModelRuntimeError)
        -> ModelMeasurement
    {
        let baseline = footprint()
        let peak = Locked(baseline)
        let footprint = footprint
        let interval = interval
        let sampling = Task {
            while !Task.isCancelled {
                let now = footprint()
                peak.withValue { $0 = max($0, now) }
                try? await Task.sleep(for: interval)
            }
        }
        let cpuBefore = cpuSeconds()
        let energyBefore = energy()
        let speed: EngineSpeed
        do throws(ModelRuntimeError) {
            speed = try await measure(model, backend)
        } catch {
            sampling.cancel()
            throw error
        }
        let cpu = cpuSeconds() - cpuBefore
        let energyAfter = energy()
        sampling.cancel()
        let end = self.footprint()
        peak.withValue { $0 = max($0, end) }
        let tokens = max(1, speed.prefillTokens + speed.decodeTokens)
        let drawn = energyBefore.flatMap { before in
            energyAfter.map {
                TaskEnergy(cpuJoules: $0.cpuJoules - before.cpuJoules, gpuSeconds: $0.gpuSeconds - before.gpuSeconds)
            }
        }
        let variant = model.spec.variant
        return ModelMeasurement(
            backend: backend, runtime: variant.runtime, quantization: variant.quantization, window: model.window,
            device: device.model, system: device.system, prefillTokensPerSecond: speed.prefillTokensPerSecond,
            decodeTokensPerSecond: speed.decodeTokensPerSecond, timeToFirstToken: speed.timeToFirstToken,
            peakMemoryBytes: max(0, peak.value - baseline), cpuSecondsPerToken: cpu / Double(tokens),
            cpuJoulesPerToken: drawn.map { $0.cpuJoules / Double(tokens) },
            gpuSecondsPerToken: drawn.map { $0.gpuSeconds / Double(tokens) }, measuredAt: now())
    }

    /// The memory the app takes now, as the system counts it against the
    /// app: its footprint, the GPU's buffers included.
    @Sendable public static func footprint() -> Int64 {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
        let result = withUnsafeMutablePointer(to: &info) { pointer in
            pointer.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return result == KERN_SUCCESS ? Int64(clamping: info.phys_footprint) : 0
    }

    /// The CPU time the app has used, in seconds, across its cores.
    @Sendable public static func cpuSeconds() -> Double {
        var usage = rusage()
        guard getrusage(RUSAGE_SELF, &usage) == 0 else { return 0 }
        func seconds(_ time: timeval) -> Double { Double(time.tv_sec) + Double(time.tv_usec) / 1_000_000 }
        return seconds(usage.ru_utime) + seconds(usage.ru_stime)
    }

    /// What the app has drawn so far: the CPU's energy, which the system
    /// reads from the chip's own counters, and the GPU's busy time. Nil on
    /// an Intel Mac, which doesn't count the energy.
    @Sendable public static func energy() -> TaskEnergy? {
        #if arch(arm64)
            var info = task_power_info_v2_data_t()
            var count = mach_msg_type_number_t(
                MemoryLayout<task_power_info_v2_data_t>.size / MemoryLayout<natural_t>.size)
            let result = withUnsafeMutablePointer(to: &info) { pointer in
                pointer.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                    task_info(mach_task_self_, task_flavor_t(TASK_POWER_INFO_V2), $0, &count)
                }
            }
            guard result == KERN_SUCCESS else { return nil }
            return TaskEnergy(
                cpuJoules: Double(info.task_energy) / 1_000_000_000,
                gpuSeconds: Double(info.gpu_energy.task_gpu_utilisation) / 1_000_000_000)
        #else
            return nil
        #endif
    }
}
