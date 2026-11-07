/// How a downloaded model runs best: where, at what precision, and with
/// which of its runtime's speedups. The catalog sets one for each model it
/// offers, from measurements on devices; a pasted model gets `standard`,
/// which every LiteRT-LM model runs with.
public struct ModelProfile: Hashable, Codable, Sendable {
    /// Where it runs while the app is on screen, if it fits there. In the
    /// background on iPhone it runs on the CPU, the only place it may.
    public var onScreen: ModelBackend
    /// The precision of its activations on the GPU.
    public var gpuActivations: Activations
    /// Where its vision encoder runs, for a model that reads photos.
    public var vision: Encoder
    /// Where its audio encoder runs, for a model that listens.
    public var audio: Encoder
    /// Whether a small drafter in the model's file guesses the next few
    /// tokens for the model to check at once, which writes faster.
    public var speculation: Speculation
    /// The threads it uses on the CPU; nil for the runtime's choice.
    public var cpuThreads: Int?
    /// How it picks each token; nil for the model's own settings.
    public var sampling: Sampling?

    public init(
        onScreen: ModelBackend = .gpu, gpuActivations: Activations = .half, vision: Encoder = .withModel,
        audio: Encoder = .cpu, speculation: Speculation = .off, cpuThreads: Int? = nil, sampling: Sampling? = nil
    ) {
        self.onScreen = onScreen
        self.gpuActivations = gpuActivations
        self.vision = vision
        self.audio = audio
        self.speculation = speculation
        self.cpuThreads = cpuThreads
        self.sampling = sampling
    }

    /// What a model runs with when nothing's known about it: the GPU at
    /// half precision, the runtime's own sampling, no drafter.
    public static let standard = ModelProfile()

    /// The precision of a model's activations.
    public enum Activations: String, Hashable, Codable, Sendable {
        /// Quicker, and enough for most models.
        case half
        /// Slower, for a model whose answers garble at half.
        case full
    }

    /// Where an encoder runs.
    public enum Encoder: String, Hashable, Codable, Sendable {
        /// Wherever the model runs.
        case withModel
        /// On the CPU, wherever the model runs.
        case cpu
    }

    /// Whether a model's drafter runs.
    public enum Speculation: String, Hashable, Codable, Sendable {
        /// Never: a model with no drafter, or one it doesn't speed up.
        case off
        /// While it's measured to write faster than the model alone.
        case adaptive
        /// On every token.
        case on
    }

    /// How a model picks each token.
    public struct Sampling: Hashable, Codable, Sendable {
        /// The likeliest tokens it picks from.
        public var topK: Int
        /// The share of likelihood it picks from.
        public var topP: Float
        /// How far from the likeliest token it strays; zero never does.
        public var temperature: Float

        public init(topK: Int, topP: Float, temperature: Float) {
            self.topK = topK
            self.topP = topP
            self.temperature = temperature
        }

        /// The likeliest token each time, so a turn reads the same on one
        /// device.
        public static let greedy = Sampling(topK: 1, topP: 1, temperature: 0)
    }
}
