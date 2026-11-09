import Foundation
import InferenceKit
import InferenceKitSpeechText
import MLX
import Tokenizers

/// A dual autoregressive speech model and its codec on MLX, such as
/// Audio8's Persian voice: the text is normalized and cut into pieces for
/// its language, each piece becomes a prompt, the model writes its codes
/// and the codec makes them audio, with a short pause between pieces.
/// Everything loads on first use.
public actor DualAREngine: SpeechEngine {
    private let model: InstalledModel
    private let sampling: Sampling
    private let maxFrames: Int
    private let seed: UInt64?
    private var loaded: Loaded?

    /// - Parameters:
    ///   - model: The downloaded model: config.json, the tokenizer's
    ///     files, model.safetensors and codec.pth.
    ///   - seed: Makes the speech repeat for the same text, for tests;
    ///     nil varies it.
    public init(model: InstalledModel, seed: UInt64? = nil) {
        self.model = model
        sampling = Sampling()
        maxFrames = 1024
        self.seed = seed
    }

    init(model: InstalledModel, sampling: Sampling, maxFrames: Int, seed: UInt64?) {
        self.model = model
        self.sampling = sampling
        self.maxFrames = maxFrames
        self.seed = seed
    }

    public func synthesize(
        _ text: String, language: String, voice: String, audio: @escaping @Sendable (SpeechAudio) -> Void
    ) async throws(SpeechError) {
        let pieces = SpokenText.pieces(of: SpokenText.normalized(text, language: language), language: language)
            .filter { !$0.allSatisfy(\.isWhitespace) }
        guard !pieces.isEmpty else { throw .nothingToSay }
        let parts = try await load()
        let rate = Double(parts.model.config.codecSampleRate)
        for (index, piece) in pieces.enumerated() {
            guard !Task.isCancelled else { return }
            if index > 0 {
                audio(SpeechAudio(samples: [Float](repeating: 0, count: Int(rate * 0.18)), sampleRate: rate))
            }
            let codes = parts.model.generate(
                parts.prompt(piece), sampling: sampling, maxFrames: maxFrames, seed: seed.map { $0 &+ UInt64(index) },
                stop: { Task.isCancelled })
            guard !Task.isCancelled else { return }
            guard !(codes.first?.isEmpty ?? true) else {
                throw .synthesisFailed(model: model.spec.name, reason: "it made no audio for “\(piece)”")
            }
            audio(SpeechAudio(samples: parts.codec.decode(codes), sampleRate: rate))
        }
    }

    /// The prompt's tokens for `text`, for checking against the model's
    /// own processor.
    func prompt(_ text: String) async throws(SpeechError) -> [Int] {
        try await load().prompt(text)
    }

    /// The codec's audio for `codes`.
    func decode(_ codes: [[Int32]]) async throws(SpeechError) -> [Float] {
        try await load().codec.decode(codes)
    }

    /// The codes the model writes for `text`.
    func codes(_ text: String) async throws(SpeechError) -> [[Int32]] {
        let parts = try await load()
        return parts.model.generate(parts.prompt(text), sampling: sampling, maxFrames: maxFrames, seed: seed)
    }

    private func load() async throws(SpeechError) -> Loaded {
        if let loaded { return loaded }
        let name = model.spec.name
        let config: DualARConfig
        do {
            let decoder = JSONDecoder()
            decoder.keyDecodingStrategy = .convertFromSnakeCase
            config = try decoder.decode(DualARConfig.self, from: Data(contentsOf: file("config.json")))
        } catch {
            throw .modelDamaged(model: name, file: "config.json", reason: "\(error)")
        }
        let tokenizer: any Tokenizer
        do {
            tokenizer = try await AutoTokenizer.from(modelFolder: model.directory)
        } catch {
            throw .modelDamaged(model: name, file: "tokenizer.json", reason: "\(error)")
        }
        let speech: DualARModel
        do {
            let arrays = try loadArrays(url: file("model.safetensors"))
            speech = try DualARModel(config: config, weights: Weights(arrays: arrays))
        } catch let missing as Weights.Missing {
            throw .modelDamaged(model: name, file: "model.safetensors", reason: "it has no \(missing.name)")
        } catch {
            throw .modelDamaged(model: name, file: "model.safetensors", reason: "\(error)")
        }
        let codec: AudioCodec
        do {
            codec = try AudioCodec(checkpoint: TorchCheckpoint(url: file("codec.pth")), config: config)
        } catch let failure as TorchCheckpoint.Failure {
            throw .modelDamaged(model: name, file: "codec.pth", reason: failure.description)
        } catch let missing as Weights.Missing {
            throw .modelDamaged(model: name, file: "codec.pth", reason: "it has no \(missing.name)")
        } catch {
            throw .modelDamaged(model: name, file: "codec.pth", reason: "\(error)")
        }
        let loaded = Loaded(model: speech, codec: codec, tokenizer: tokenizer)
        self.loaded = loaded
        return loaded
    }

    private func file(_ path: String) -> URL {
        model.directory.appending(path: path, directoryHint: .notDirectory)
    }
}

/// What a loaded engine speaks with. MLX's arrays aren't Sendable; the
/// actor is the only one to touch them.
private struct Loaded: @unchecked Sendable {
    let model: DualARModel
    let codec: AudioCodec
    let tokenizer: any Tokenizer

    /// The model's prompt for `text`, each part tokenized on its own as
    /// its processor does.
    func prompt(_ text: String) -> [Int] {
        let target = text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        return [
            "<|im_start|>system\n", "convert the provided text to speech", "<|im_end|>\n", "<|im_start|>user\n",
            target, "<|im_end|>\n", "<|im_start|>assistant\n<|voice|>",
        ].flatMap { tokenizer.encode(text: $0, addSpecialTokens: false) }
    }
}
