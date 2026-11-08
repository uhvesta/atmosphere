/// Picks where a downloaded model runs while the app is on screen.
public enum ModelPlanner {
    /// Where a model runs, and why.
    public struct Plan: Hashable, Sendable {
        public var backend: ModelBackend
        public var reason: Reason

        public init(backend: ModelBackend, reason: Reason) {
            self.backend = backend
            self.reason = reason
        }
    }

    public enum Reason: Hashable, Sendable {
        /// The person picked it.
        case picked
        /// It was the fastest that fits, as measured on this device.
        case measured(ModelMeasurement)
        /// Nothing's measured that fits, so it runs as its profile says.
        case profile
    }

    /// The backend `model` runs on, of those its runtime has: the person's
    /// pick if they made one; else, of those `measurements` that describe
    /// it here and fit in `available` bytes, the one that writes fastest,
    /// the lighter one when they're within a tenth of each other; else its
    /// profile's.
    public static func plan(
        _ model: InstalledModel, picked: ModelBackend?, measurements: [ModelMeasurement], device: String,
        system: String, fits: (Int64) -> Bool
    ) -> Plan {
        let backends = model.spec.runtime.backends
        if let picked, backends.contains(picked) { return Plan(backend: picked, reason: .picked) }
        let candidates = measurements.filter {
            $0.describes(model, device: device, system: system) && fits($0.peakMemoryBytes)
                && backends.contains($0.backend)
        }
        guard let fastest = candidates.max(by: { $0.decodeTokensPerSecond < $1.decodeTokensPerSecond }) else {
            let profile = model.spec.profile.onScreen
            return Plan(backend: backends.contains(profile) ? profile : .gpu, reason: .profile)
        }
        let close = candidates.filter { $0.decodeTokensPerSecond >= fastest.decodeTokensPerSecond * 0.9 }
        let best = close.min { lighter($0, than: $1) } ?? fastest
        return Plan(backend: best.backend, reason: .measured(best))
    }

    /// Lighter: less of the CPU, then less memory.
    private static func lighter(_ one: ModelMeasurement, than other: ModelMeasurement) -> Bool {
        if one.isLowCPU != other.isLowCPU { return one.isLowCPU }
        return one.peakMemoryBytes < other.peakMemoryBytes
    }
}

/// What's known of how one downloaded model runs here: where it runs and
/// why, the person's pick if any, and what was measured of it on this
/// device at its window, one per backend, latest first.
public struct ModelTuning: Hashable, Sendable {
    public var plan: ModelPlanner.Plan
    public var picked: ModelBackend?
    public var measurements: [ModelMeasurement]
    /// Where its runtime can run it.
    public var backends: [ModelBackend]

    public init(
        plan: ModelPlanner.Plan, picked: ModelBackend? = nil, measurements: [ModelMeasurement] = [],
        backends: [ModelBackend] = [.gpu, .cpu]
    ) {
        self.plan = plan
        self.picked = picked
        self.measurements = measurements
        self.backends = backends
    }

    /// The measurement for `backend`, if it was measured.
    public func measurement(on backend: ModelBackend) -> ModelMeasurement? {
        measurements.first { $0.backend == backend }
    }
}
