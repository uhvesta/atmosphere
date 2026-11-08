import InferenceKit
import SwiftUI

/// Where a downloaded model runs and why, how fast it ran here on each
/// backend side by side, and a way to measure it again or pick where it
/// runs. Nothing's measured until the person asks.
public struct ModelTuningView: View {
    let tuning: ModelTuning
    let isMeasuring: Bool
    let error: String?
    let measure: () -> Void
    let pick: (ModelBackend?) -> Void

    /// `measure` runs the short test on each backend; `pick` sets where it
    /// runs, or lets the app pick with nil.
    public init(
        _ tuning: ModelTuning, isMeasuring: Bool, error: String?, measure: @escaping () -> Void,
        pick: @escaping (ModelBackend?) -> Void
    ) {
        self.tuning = tuning
        self.isMeasuring = isMeasuring
        self.error = error
        self.measure = measure
        self.pick = pick
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Picker("Runs on", selection: Binding(get: { tuning.picked }, set: pick)) {
                Text("Automatic (\(tuning.plan.backend.title))").tag(ModelBackend?.none)
                ForEach(tuning.backends, id: \.self) { backend in
                    Text(backend.title).tag(ModelBackend?.some(backend))
                }
            }
            .disabled(isMeasuring)
            Text(reason).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if !tuning.measurements.isEmpty {
                results
            }
            if let error {
                Text(error).font(.caption).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: 8) {
                Button(tuning.measurements.isEmpty ? "Measure on this device" : "Measure again", action: measure)
                    .disabled(isMeasuring)
                if isMeasuring {
                    ProgressView().controlSize(.small)
                    Text("Measuring, about a minute").font(.caption).foregroundStyle(.secondary)
                }
            }
        }
        .buttonStyle(.borderless)
    }

    private var reason: String {
        let backend = tuning.plan.backend.title
        switch tuning.plan.reason {
        case .picked:
            return String(localized: "Runs on the \(backend), as you picked.")
        case .measured(let measurement):
            let window = measurement.window / 1_024
            let speed = Self.speed(measurement.decodeTokensPerSecond)
            let memory = Self.bytes(measurement.peakMemoryBytes)
            return measurement.isLowCPU
                ? String(
                    localized: """
                        Runs on the \(backend): fastest here at \(window)K tokens, \(speed) in \(memory), \
                        leaving the CPU free.
                        """)
                : String(localized: "Runs on the \(backend): fastest here at \(window)K tokens, \(speed) in \(memory).")
        case .profile:
            return String(
                localized: "Runs on the \(backend), where it does best on most devices. Measure it to see this one.")
        }
    }

    /// Each backend measured, side by side.
    @ViewBuilder private var results: some View {
        Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 4) {
            GridRow {
                Text(verbatim: "")
                ForEach(tuning.backends, id: \.self) { Text($0.title).bold() }
            }
            row("Writes") { Self.speed($0.decodeTokensPerSecond) }
            row("Reads") { Self.speed($0.prefillTokensPerSecond) }
            row("First word") { Self.seconds($0.timeToFirstToken) }
            row("Memory") { Self.bytes($0.peakMemoryBytes) }
            row("CPU time per token") { Self.seconds($0.cpuSecondsPerToken, digits: 3) }
            if countsEnergy {
                row("Battery per token") { $0.cpuJoulesPerToken.map(Self.millijoules) ?? "–" }
                row("GPU time per token") { $0.gpuSecondsPerToken.map(Self.milliseconds) ?? "–" }
            }
        }
        .font(.caption.monospacedDigit())
        Group {
            if countsEnergy {
                Text(
                    """
                    CPU time is the CPU's seconds, across its cores. Battery is what the CPU drew: the system \
                    doesn't tell an app what the GPU drew, so its busy time is shown instead.
                    """)
            } else {
                Text("CPU time is the CPU's seconds, across its cores.")
            }
        }
        .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
    }

    /// Whether the system counted the energy the CPU drew.
    private var countsEnergy: Bool { tuning.measurements.contains { $0.cpuJoulesPerToken != nil } }

    private func row(_ title: LocalizedStringKey, _ value: @escaping (ModelMeasurement) -> String) -> some View {
        GridRow {
            Text(title).foregroundStyle(.secondary)
            ForEach(tuning.backends, id: \.self) { backend in
                Text(tuning.measurement(on: backend).map(value) ?? "–")
            }
        }
    }

    private static func speed(_ tokensPerSecond: Double) -> String {
        String(localized: "\(tokensPerSecond.formatted(.number.precision(.fractionLength(0)))) tok/s")
    }

    private static func seconds(_ seconds: Double, digits: Int = 1) -> String {
        String(localized: "\(seconds.formatted(.number.precision(.fractionLength(digits)))) s")
    }

    private static func millijoules(_ joules: Double) -> String {
        String(localized: "\((joules * 1_000).formatted(.number.precision(.fractionLength(0)))) mJ")
    }

    private static func milliseconds(_ seconds: Double) -> String {
        String(localized: "\((seconds * 1_000).formatted(.number.precision(.fractionLength(0)))) ms")
    }

    private static func bytes(_ count: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: count, countStyle: .memory)
    }
}

extension ModelBackend {
    /// GPU or CPU.
    public var title: String {
        switch self {
        case .gpu: String(localized: "GPU")
        case .cpu: String(localized: "CPU")
        }
    }
}
