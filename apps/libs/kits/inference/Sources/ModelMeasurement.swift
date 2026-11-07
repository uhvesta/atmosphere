import Foundation

/// How fast one downloaded model ran on one backend of one device, measured
/// by a short fixed prompt when the person asked.
public struct ModelMeasurement: Hashable, Codable, Sendable {
    public var backend: ModelBackend
    /// The variant measured.
    public var runtime: RuntimeKind
    public var quantization: String
    /// The window it was measured at.
    public var window: Int
    /// The device and system it ran on, such as iPhone18,1 and 27.0. A
    /// measurement from another one says little about this one.
    public var device: String
    public var system: String
    /// Tokens read per second.
    public var prefillTokensPerSecond: Double
    /// Tokens written per second.
    public var decodeTokensPerSecond: Double
    /// Seconds until the first token was written.
    public var timeToFirstToken: Double
    /// The most memory the app took while it ran, past what it took before
    /// the model loaded.
    public var peakMemoryBytes: Int64
    /// The CPU's seconds, across its cores, for each token read or written.
    public var cpuSecondsPerToken: Double
    /// The energy the CPU drew from the battery for each token, in joules,
    /// as the system counts it against the app; nil where it doesn't. The
    /// GPU's isn't in it: the system doesn't tell an app what it drew.
    public var cpuJoulesPerToken: Double?
    /// The GPU's busy time for each token, in seconds, which stands in for
    /// what it drew; nil where the system doesn't say.
    public var gpuSecondsPerToken: Double?
    public var measuredAt: Date

    public init(
        backend: ModelBackend, runtime: RuntimeKind, quantization: String = "", window: Int, device: String,
        system: String, prefillTokensPerSecond: Double, decodeTokensPerSecond: Double, timeToFirstToken: Double,
        peakMemoryBytes: Int64, cpuSecondsPerToken: Double, cpuJoulesPerToken: Double? = nil,
        gpuSecondsPerToken: Double? = nil, measuredAt: Date
    ) {
        self.backend = backend
        self.runtime = runtime
        self.quantization = quantization
        self.window = window
        self.device = device
        self.system = system
        self.prefillTokensPerSecond = prefillTokensPerSecond
        self.decodeTokensPerSecond = decodeTokensPerSecond
        self.timeToFirstToken = timeToFirstToken
        self.peakMemoryBytes = peakMemoryBytes
        self.cpuSecondsPerToken = cpuSecondsPerToken
        self.cpuJoulesPerToken = cpuJoulesPerToken
        self.gpuSecondsPerToken = gpuSecondsPerToken
        self.measuredAt = measuredAt
    }

    /// Whether it says how `model` runs on `device` with `system`: the same
    /// variant at the same window, there.
    public func describes(_ model: InstalledModel, device: String, system: String) -> Bool {
        let variant = model.spec.variant
        return runtime == variant.runtime && quantization == variant.quantization && window == model.window
            && self.device == device && self.system == system
    }

    /// Whether it took little of the CPU: under a hundredth of a second per
    /// token, as on a GPU that does the work.
    public var isLowCPU: Bool { cpuSecondsPerToken < 0.01 }
}

/// What an engine measured of one short run, before the app adds memory
/// and CPU.
public struct EngineSpeed: Hashable, Sendable {
    public var prefillTokens: Int
    public var decodeTokens: Int
    public var prefillTokensPerSecond: Double
    public var decodeTokensPerSecond: Double
    public var timeToFirstToken: Double

    public init(
        prefillTokens: Int, decodeTokens: Int, prefillTokensPerSecond: Double, decodeTokensPerSecond: Double,
        timeToFirstToken: Double
    ) {
        self.prefillTokens = prefillTokens
        self.decodeTokens = decodeTokens
        self.prefillTokensPerSecond = prefillTokensPerSecond
        self.decodeTokensPerSecond = decodeTokensPerSecond
        self.timeToFirstToken = timeToFirstToken
    }
}
