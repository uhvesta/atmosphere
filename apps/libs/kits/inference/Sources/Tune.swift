import Foundation

/// One way to run a downloaded model, to compare with the others: one of
/// its variants, such as GGUF on llama.cpp, with engine options.
public struct TuneCandidate: Hashable, Codable, Sendable, Identifiable {
    public var model: ModelID
    public var runtime: RuntimeKind
    public var quantization: String
    public var options: EngineOptions

    public init(model: ModelID, runtime: RuntimeKind, quantization: String, options: EngineOptions) {
        self.model = model
        self.runtime = runtime
        self.quantization = quantization
        self.options = options
    }

    public var id: Self { self }

    /// The setup it runs as, for the planner.
    public var setup: TunedSetup { TunedSetup(runtime: runtime, quantization: quantization, options: options) }

    /// What its results are kept under on `device` with `system`.
    public func key(device: String, system: String) -> TuneKey {
        TuneKey(device: device, system: system, candidate: self)
    }
}

/// What a candidate's results are kept under: what ran, and on which
/// device and system, as a result from another one says little of this one.
public struct TuneKey: Hashable, Codable, Sendable {
    public var device: String
    public var system: String
    public var candidate: TuneCandidate

    public init(device: String, system: String, candidate: TuneCandidate) {
        self.device = device
        self.system = system
        self.candidate = candidate
    }
}

/// How one candidate ran: its speed, memory and CPU, and how close its
/// answers came to the reference's.
public struct TuneResult: Hashable, Codable, Sendable {
    public var key: TuneKey
    public var measurement: ModelMeasurement
    public var quality: TuneQuality?

    public init(key: TuneKey, measurement: ModelMeasurement, quality: TuneQuality? = nil) {
        self.key = key
        self.measurement = measurement
        self.quality = quality
    }

    /// When it was measured.
    public var measuredAt: Date { measurement.measuredAt }
}

/// How close a candidate's answers to the quality prompts came to the
/// reference's, word for word.
public struct TuneQuality: Hashable, Codable, Sendable {
    /// From 0, no word in common, to 1, the same answers.
    public var agreement: Double
    /// Whether it's the reference, which agrees with itself.
    public var isReference: Bool
    /// The reference it was compared with.
    public var reference: TuneCandidate

    public init(agreement: Double, isReference: Bool, reference: TuneCandidate) {
        self.agreement = agreement
        self.isReference = isReference
        self.reference = reference
    }

    /// How alike two answers are: the words they have in the same order,
    /// over the longer one's words, ignoring case and punctuation. Two
    /// empty answers are alike.
    public static func agreement(_ answer: String, _ reference: String) -> Double {
        let one = words(answer)
        let other = words(reference)
        let longer = max(one.count, other.count)
        guard longer > 0 else { return 1 }
        return Double(commonSubsequence(one, other)) / Double(longer)
    }

    /// The mean agreement of each answer with the reference's to the same
    /// prompt; nil when there are none.
    public static func agreement(_ answers: [String], _ references: [String]) -> Double? {
        let pairs = zip(answers, references).map { agreement($0, $1) }
        guard !pairs.isEmpty else { return nil }
        return pairs.reduce(0, +) / Double(pairs.count)
    }

    private static func words(_ text: String) -> [Substring] {
        text.lowercased().split { !$0.isLetter && !$0.isNumber }
    }

    private static func commonSubsequence(_ one: [Substring], _ other: [Substring]) -> Int {
        guard !one.isEmpty, !other.isEmpty else { return 0 }
        var previous = [Int](repeating: 0, count: other.count + 1)
        for word in one {
            var row = [Int](repeating: 0, count: other.count + 1)
            for (index, each) in other.enumerated() {
                row[index + 1] = word == each ? previous[index] + 1 : max(previous[index + 1], row[index])
            }
            previous = row
        }
        return previous[other.count]
    }
}

/// The short prompts each candidate answers for its quality, picking the
/// likeliest word each time and without thinking, so the same setup writes
/// the same answer.
public enum TunePrompts {
    public static let standard = [
        "In one sentence, why does ice float on water?",
        "List three things to pack for a weekend hike, separated by commas.",
        "What is 17 times 23? Answer with the number only.",
        "Rewrite in plain words: The meeting has been postponed until further notice.",
    ]

    /// The words of each answer it keeps, so a candidate that rambles
    /// isn't timed for it.
    public static let answerWords = 40
}

/// Why a candidate has no result.
public enum TuneFailure: Error, Hashable, Sendable, CustomStringConvertible {
    /// Its options can't be used as set.
    case invalidOptions(EngineOptionsError)
    /// Its variant isn't downloaded.
    case notDownloaded
    /// Its engine failed, such as for want of memory.
    case engine(ModelRuntimeError)

    public var description: String {
        switch self {
        case .invalidOptions(let error): error.description
        case .notDownloaded: "This format isn't downloaded. Download it to compare it."
        case .engine(let error): error.description
        }
    }
}

/// What came of one candidate in a run.
public enum TuneOutcome: Hashable, Sendable {
    case measured(TuneResult)
    case failed(TuneFailure)
    /// The run was stopped before it got to it.
    case skipped
}

/// What a model is tuned for: each can run its own setup.
public enum TunePurpose: String, Hashable, Codable, Sendable, CaseIterable {
    /// Talking with the person, where writing fast matters most.
    case chat
    /// Short tasks such as naming a chat, where the first word matters most.
    case quick
}

/// How the person picked for a model to run for a purpose.
public struct TunedSetup: Hashable, Codable, Sendable {
    public var runtime: RuntimeKind
    public var quantization: String
    public var options: EngineOptions

    public init(runtime: RuntimeKind, quantization: String, options: EngineOptions) {
        self.runtime = runtime
        self.quantization = quantization
        self.options = options
    }
}

extension ModelUseCase {
    /// Chat, unless the use says.
    public var purpose: TunePurpose { .chat }
}

/// The candidates for a downloaded model: each variant on the device on
/// each backend its engine has, at `window`, with the engine's own
/// settings. The person adds others by changing the options.
public func tuneCandidates(for installed: [InstalledModel], window: Int) -> [TuneCandidate] {
    installed.flatMap { model in
        let variant = model.spec.variant
        return variant.runtime.backends.map { backend in
            TuneCandidate(
                model: model.id, runtime: variant.runtime, quantization: variant.quantization,
                options: EngineOptions(backend: backend, window: window))
        }
    }
}

/// The candidate whose answers the others are compared with: the most
/// precise of them, the first of those that tie. It's the most precise
/// setup on the device, not the model at full precision, which isn't
/// downloaded.
public func tuneReference(of candidates: [TuneCandidate]) -> TuneCandidate? {
    candidates.enumerated().min { one, other in
        (one.element.options.imprecision, one.offset) < (other.element.options.imprecision, other.offset)
    }?.element
}
