import InferenceKit
import SwiftUI

/// What the Tune screen shows of one model: the setups to compare, what
/// was measured of each here, the setup picked for each purpose, and the
/// formats it has that aren't downloaded.
public struct TuneScreenState {
    public var model: ModelSpec
    public var candidates: [TuneCandidate]
    /// The latest result of each candidate measured on this device.
    public var results: [TuneCandidate: TuneResult]
    /// Why a candidate in the last run has no result.
    public var failures: [TuneCandidate: String]
    /// The candidate running now, while a run is on.
    public var running: TuneCandidate?
    public var isRunning: Bool
    public var setups: [TunePurpose: TunedSetup]
    public var otherFormats: [ModelVariant]
    /// The format downloading now, and why the last one didn't.
    public var downloading: ModelVariant?
    public var downloadError: String?
    /// The index of the candidate whose settings are open.
    public var editing: Int?

    public init(
        model: ModelSpec, candidates: [TuneCandidate], results: [TuneCandidate: TuneResult],
        failures: [TuneCandidate: String], running: TuneCandidate?, isRunning: Bool,
        setups: [TunePurpose: TunedSetup], otherFormats: [ModelVariant], downloading: ModelVariant?,
        downloadError: String?, editing: Int?
    ) {
        self.model = model
        self.candidates = candidates
        self.results = results
        self.failures = failures
        self.running = running
        self.isRunning = isRunning
        self.setups = setups
        self.otherFormats = otherFormats
        self.downloading = downloading
        self.downloadError = downloadError
        self.editing = editing
    }
}

/// What the person can do on the Tune screen.
public struct TuneScreenActions {
    /// Runs every candidate, one after another.
    public var tune: () -> Void
    public var stop: () -> Void
    /// Opens the settings of the candidate at an index, or closes them.
    public var edit: (Int?) -> Void
    /// Sets the options of the candidate at an index.
    public var change: (EngineOptions, Int) -> Void
    /// Adds a copy of the candidate at an index, to change.
    public var duplicate: (Int) -> Void
    public var remove: (Int) -> Void
    /// Runs the model as a setup for a purpose, or as the app picks with nil.
    public var pick: (TunedSetup?, TunePurpose) -> Void
    public var download: (ModelVariant) -> Void

    public init(
        tune: @escaping () -> Void, stop: @escaping () -> Void, edit: @escaping (Int?) -> Void,
        change: @escaping (EngineOptions, Int) -> Void,
        duplicate: @escaping (Int) -> Void, remove: @escaping (Int) -> Void,
        pick: @escaping (TunedSetup?, TunePurpose) -> Void, download: @escaping (ModelVariant) -> Void
    ) {
        self.tune = tune
        self.stop = stop
        self.edit = edit
        self.change = change
        self.duplicate = duplicate
        self.remove = remove
        self.pick = pick
        self.download = download
    }
}

/// Compares the ways a downloaded model can run here, side by side: each
/// format it's downloaded in, on each engine and backend, with the
/// engine's settings changed. The person picks one for chats and one for
/// quick tasks. Nothing runs until they tap Tune for this device, and
/// nothing downloads until they tap a format.
public struct TuneScreen: View {
    let state: TuneScreenState
    let actions: TuneScreenActions

    public init(_ state: TuneScreenState, actions: TuneScreenActions) {
        self.state = state
        self.actions = actions
    }

    public var body: some View {
        Form {
            run
            if !state.results.isEmpty || !state.failures.isEmpty {
                results
            }
            picks
            setups
            if !state.otherFormats.isEmpty {
                formats
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Tune \(state.model.name)")
    }

    // MARK: Running

    @ViewBuilder private var run: some View {
        Section {
            if state.isRunning {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(progress).font(.callout)
                    Spacer()
                    Button("Stop", role: .cancel, action: actions.stop)
                }
            } else {
                Button(state.results.isEmpty ? "Tune for this device" : "Tune again", action: actions.tune)
                    .disabled(state.candidates.isEmpty)
            }
        } footer: {
            Text(
                """
                Runs each setup below on a short test, one after another: about a minute each. The model \
                that's loaded now is unloaded first.
                """)
        }
    }

    private var progress: String {
        guard let running = state.running, let index = state.candidates.firstIndex(of: running) else {
            return String(localized: "Measuring…")
        }
        return String(
            localized: "Measuring \(index + 1) of \(state.candidates.count): \(running.title(in: state.model))")
    }

    // MARK: Results

    @ViewBuilder private var results: some View {
        Section {
            ScrollView(.horizontal) {
                Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
                    GridRow {
                        Text(verbatim: "")
                        ForEach(state.candidates) { Text($0.title(in: state.model)).bold() }
                    }
                    row("Reads") { Self.speed($0.measurement.prefillTokensPerSecond) }
                    row("Writes") { Self.speed($0.measurement.decodeTokensPerSecond) }
                    row("First word") { Self.seconds($0.measurement.timeToFirstToken) }
                    row("Peak memory") { Self.bytes($0.measurement.peakMemoryBytes) }
                    row("CPU per token") { Self.seconds($0.measurement.cpuSecondsPerToken, digits: 3) }
                    row("Matches") { result in
                        guard let quality = result.quality else { return "–" }
                        return quality.isReference
                            ? String(localized: "Reference")
                            : quality.agreement.formatted(.percent.precision(.fractionLength(0)))
                    }
                    GridRow {
                        Text("Measured").foregroundStyle(.secondary)
                        ForEach(state.candidates) { candidate in
                            if let result = state.results[candidate] {
                                Text(result.measuredAt, format: .dateTime.month().day().hour().minute())
                            } else {
                                Text(verbatim: "–")
                            }
                        }
                    }
                }
                .font(.caption.monospacedDigit())
                .padding(.vertical, 4)
            }
            ForEach(state.candidates.filter { state.failures[$0] != nil }) { candidate in
                if let failure = state.failures[candidate] {
                    Text("\(candidate.title(in: state.model)): \(failure)").font(.caption).foregroundStyle(.red)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        } header: {
            Text("Results")
        } footer: {
            VStack(alignment: .leading, spacing: 6) {
                Text(
                    """
                    Reads is how fast it takes in what's sent (prefill); writes is how fast it answers \
                    (decode). First word is the wait before the answer starts.
                    """)
                Text(
                    """
                    Matches is how many of its words, in order, match the most precise setup's on four short \
                    prompts. That's the most precise setup on this device, not the full-size model, which \
                    isn't downloaded: a cheap check that a setup doesn't garble, not a score of how good it is.
                    """)
            }
        }
    }

    private func row(_ title: LocalizedStringKey, _ value: @escaping (TuneResult) -> String) -> some View {
        GridRow {
            Text(title).foregroundStyle(.secondary)
            ForEach(state.candidates) { candidate in
                if let result = state.results[candidate] {
                    Text(value(result))
                } else if state.failures[candidate] != nil {
                    Text("Failed").foregroundStyle(.red)
                } else {
                    Text(verbatim: "–")
                }
            }
        }
    }

    // MARK: Picks

    @ViewBuilder private var picks: some View {
        Section {
            ForEach(TunePurpose.allCases, id: \.self) { purpose in
                Picker(
                    purpose.title,
                    selection: Binding(get: { state.setups[purpose] }, set: { actions.pick($0, purpose) })
                ) {
                    Text("Automatic").tag(TunedSetup?.none)
                    ForEach(choices(for: purpose), id: \.self) { setup in
                        Text(setup.title(in: state.model)).tag(TunedSetup?.some(setup))
                    }
                }
                .disabled(state.isRunning)
            }
        } header: {
            Text("Use")
        } footer: {
            Text(
                """
                Chats want fast writing; quick tasks, such as naming a conversation, want the first word \
                soon. Automatic runs it where it measured fastest.
                """)
        }
    }

    /// Each candidate's setup, and the one picked if it's no longer listed.
    private func choices(for purpose: TunePurpose) -> [TunedSetup] {
        let listed = state.candidates.map(\.setup)
        guard let picked = state.setups[purpose], !listed.contains(picked) else { return listed }
        return listed + [picked]
    }

    // MARK: Setups

    @ViewBuilder private var setups: some View {
        Section {
            ForEach(Array(state.candidates.enumerated()), id: \.offset) { index, candidate in
                let isOpen = state.editing == index
                Button {
                    actions.edit(isOpen ? nil : index)
                } label: {
                    HStack {
                        Text(candidate.title(in: state.model)).foregroundStyle(Color.primary)
                        Spacer()
                        Image(systemName: isOpen ? "chevron.down" : "chevron.right")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.borderless)
                .disabled(state.isRunning)
                if isOpen {
                    SetupEditor(candidate: candidate, model: state.model, change: { actions.change($0, index) })
                        .disabled(state.isRunning)
                    HStack {
                        Button("Duplicate") { actions.duplicate(index) }
                        Spacer()
                        Button("Remove", role: .destructive) { actions.remove(index) }
                            .accessibilityLabel(Text("Remove \(candidate.title(in: state.model))"))
                            .disabled(state.candidates.count < 2)
                    }
                    .buttonStyle(.borderless)
                    .disabled(state.isRunning)
                }
            }
        } header: {
            Text("Setups")
        } footer: {
            VStack(alignment: .leading, spacing: 6) {
                Text(
                    """
                    A KV cache at 8 or 4 bits keeps less memory for each token read, and its answers drift \
                    further from a 16-bit cache's.
                    """)
                Text(
                    """
                    Flash attention works out attention in one step, which takes less memory; a cache under \
                    16 bits needs it. Layers not on the GPU run on the CPU.
                    """)
            }
        }
    }

    // MARK: Formats

    @ViewBuilder private var formats: some View {
        Section {
            ForEach(state.otherFormats, id: \.self) { variant in
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(variant.title)
                        if let size = variant.source?.totalSize {
                            Text(Self.bytes(size)).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    Spacer()
                    if state.downloading == variant {
                        ProgressView().controlSize(.small)
                    } else {
                        Button("Download") { actions.download(variant) }
                            .accessibilityLabel(Text("Download \(variant.title)"))
                            .disabled(state.downloading != nil || state.isRunning)
                    }
                }
            }
            if let error = state.downloadError {
                Text(error).font(.caption).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true)
            }
        } header: {
            Text("Other formats")
        } footer: {
            Text("Only downloaded formats are compared. Download one to add it; nothing downloads until you tap.")
        }
    }

    // MARK: Formatting

    private static func speed(_ tokensPerSecond: Double) -> String {
        String(localized: "\(tokensPerSecond.formatted(.number.precision(.fractionLength(0)))) tok/s")
    }

    private static func seconds(_ seconds: Double, digits: Int = 1) -> String {
        String(localized: "\(seconds.formatted(.number.precision(.fractionLength(digits)))) s")
    }

    static func bytes(_ count: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: count, countStyle: .memory)
    }
}

/// The settings of one setup its engine takes, each left at the engine's
/// own unless the person sets it.
private struct SetupEditor: View {
    let candidate: TuneCandidate
    let model: ModelSpec
    let change: (EngineOptions) -> Void

    var body: some View {
        let settings = candidate.runtime.settings
        if candidate.runtime.backends.count > 1 {
            Picker("Runs on", selection: binding(\.backend)) {
                ForEach(candidate.runtime.backends, id: \.self) { Text($0.title).tag($0) }
            }
        }
        Picker("Window", selection: binding(\.window)) {
            ForEach(model.windows, id: \.self) { Text("\($0 / 1_024)K tokens").tag($0) }
        }
        if settings.contains(.cpuThreads) {
            option("CPU threads", \.cpuThreads, [1, 2, 4, 6, 8]) { "\($0)" }
        }
        if settings.contains(.gpuLayers), candidate.options.backend == .gpu {
            option("GPU layers", \.gpuLayers, [0, 8, 16, 24, 32], standard: String(localized: "All")) { "\($0)" }
        }
        if settings.contains(.batchTokens) {
            option("Batch", \.batchTokens, [128, 256, 512, 1_024, 2_048]) { "\($0)" }
        }
        if settings.contains(.microBatchTokens) {
            option("Micro-batch", \.microBatchTokens, [64, 128, 256, 512]) { "\($0)" }
        }
        if settings.contains(.kvCache) {
            option("KV cache", \.kvCache, EngineOptions.KVCache.allCases) { $0.title }
        }
        if settings.contains(.flashAttention) {
            option("Flash attention", \.flashAttention, [true, false]) {
                $0 ? String(localized: "On") : String(localized: "Off")
            }
        }
        if settings.contains(.gpuActivations), candidate.options.backend == .gpu {
            option("GPU precision", \.gpuActivations, [.half, .full]) {
                $0 == .half ? String(localized: "16 bits") : String(localized: "32 bits")
            }
        }
        if settings.isEmpty {
            Text("\(candidate.runtime.name) has no settings of its own to change.")
                .font(.caption).foregroundStyle(.secondary)
        }
        if let problem {
            Text(problem).font(.caption).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true)
        }
    }

    /// Why the options can't run as set.
    private var problem: String? {
        do throws(EngineOptionsError) {
            try candidate.options.validate(for: model, on: candidate.runtime)
            return nil
        } catch {
            return error.description
        }
    }

    private func binding<Value>(_ key: WritableKeyPath<EngineOptions, Value>) -> Binding<Value> {
        Binding(
            get: { candidate.options[keyPath: key] },
            set: { value in
                var options = candidate.options
                options[keyPath: key] = value
                change(options)
            })
    }

    private func option<Value: Hashable>(
        _ title: LocalizedStringKey, _ key: WritableKeyPath<EngineOptions, Value?>, _ values: [Value],
        standard: String = String(localized: "Engine's own"), label: @escaping (Value) -> String
    ) -> some View {
        Picker(title, selection: binding(key)) {
            Text(standard).tag(Value?.none)
            ForEach(values, id: \.self) { Text(label($0)).tag(Value?.some($0)) }
        }
    }
}

extension TuneCandidate {
    /// Its engine and backend, and the settings it changes from `model`'s
    /// own, such as "llama.cpp · GPU · KV 4-bit".
    public func title(in model: ModelSpec) -> String { setup.title(in: model) }
}

extension TunedSetup {
    /// Its engine and backend, and the settings it changes from `model`'s
    /// own.
    public func title(in model: ModelSpec) -> String {
        var parts = [runtime.name, options.backend.title]
        if options.window != model.contextTokens {
            parts.append(String(localized: "\(options.window / 1_024)K tokens"))
        }
        if let threads = options.cpuThreads { parts.append(String(localized: "\(threads) threads")) }
        if let layers = options.gpuLayers { parts.append(String(localized: "\(layers) GPU layers")) }
        if let batch = options.batchTokens { parts.append(String(localized: "batch \(batch)")) }
        if let micro = options.microBatchTokens { parts.append(String(localized: "micro-batch \(micro)")) }
        if let cache = options.kvCache { parts.append(String(localized: "KV \(cache.title)")) }
        if let flash = options.flashAttention {
            parts.append(flash ? String(localized: "flash on") : String(localized: "flash off"))
        }
        if options.gpuActivations == .full { parts.append(String(localized: "32-bit GPU")) }
        return parts.joined(separator: " · ")
    }
}

extension EngineOptions.KVCache {
    /// Such as 8-bit.
    public var title: String {
        switch self {
        case .f16: String(localized: "16-bit")
        case .q8: String(localized: "8-bit")
        case .q4: String(localized: "4-bit")
        }
    }
}

extension TunePurpose {
    /// Such as For chats.
    public var title: String {
        switch self {
        case .chat: String(localized: "For chats")
        case .quick: String(localized: "For quick tasks")
        }
    }
}

extension ModelVariant {
    /// Its file format and engine, such as GGUF on llama.cpp, Q4_K_M.
    public var title: String {
        let format =
            switch runtime {
            case .liteRT: ".litertlm"
            case .llama: "GGUF"
            case .mlx: "Safetensors"
            case .coreML: ".mlmodel"
            case .apple: ""
            }
        let name = String(localized: "\(format) on \(runtime.name)")
        return quantization.isEmpty ? name : "\(name), \(quantization)"
    }
}
