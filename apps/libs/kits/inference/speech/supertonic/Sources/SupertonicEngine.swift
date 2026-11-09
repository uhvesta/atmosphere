@preconcurrency import CoreML
import Foundation
import InferenceKit

/// Supertonic 3 on Core ML: for each piece of the text, the duration
/// predictor says how long it takes to say, the text encoder reads it, the
/// vector estimator turns noise into speech's latent in `steps` passes of
/// flow matching, and the vocoder makes the audio. Each package compiles
/// once, into Caches, and loads on first use.
public actor SupertonicEngine: SpeechEngine {
    private let model: InstalledModel
    private let steps: Int
    private let speed: Float
    private var loaded: Loaded?

    /// - Parameters:
    ///   - model: The downloaded model.
    ///   - steps: The vector estimator's passes: more is cleaner and slower.
    ///   - speed: How fast it speaks; Supertone's default is 1.05.
    public init(model: InstalledModel, steps: Int = 8, speed: Float = 1.05) {
        self.model = model
        self.steps = steps
        self.speed = speed
    }

    public func synthesize(
        _ text: String, language: String, voice: String, audio: @escaping @Sendable (SpeechAudio) -> Void
    ) async throws(SpeechError) {
        let parts = try await load()
        let style = try parts.style(voice.isEmpty ? "F1" : voice, of: model)
        let language = VoiceModel.base(of: language)
        let pieces = SupertonicText.encoded(text, language: language, indexer: parts.indexer)
            .filter { !$0.isEmpty }
        guard !pieces.isEmpty else { throw .nothingToSay }
        let rate = parts.settings.sampleRate
        for (index, ids) in pieces.enumerated() {
            guard !Task.isCancelled else { return }
            if index > 0 {
                audio(SpeechAudio(samples: [Float](repeating: 0, count: Int(rate * 0.3)), sampleRate: rate))
            }
            let samples = try await speak(ids, style: style, parts: parts)
            guard !Task.isCancelled else { return }
            audio(SpeechAudio(samples: samples, sampleRate: rate))
        }
    }

    private func speak(_ ids: [Int32], style: Style, parts: Loaded) async throws(SpeechError) -> [Float] {
        let name = model.spec.name
        let width = SupertonicText.width
        let textIDs = ids + [Int32](repeating: 0, count: width - ids.count)
        let textMask = (0..<width).map { $0 < ids.count ? Float(1) : 0 }
        do {
            let duration = try await parts.duration.run(
                [
                    "text_ids": .int(textIDs, [1, width]), "style_dp": .float(style.dp, [1, 8, 16]),
                    "text_mask": .float(textMask, [1, 1, width]),
                ], output: "duration")
            let seconds = (duration.first ?? 0) / speed
            let settings = parts.settings
            let samples = max(Int(seconds * Float(settings.sampleRate)), 1)
            let frame = settings.chunkSize
            let latentLength = min((samples + frame - 1) / frame, 512)
            let length = max(latentLength, 17)
            let dimension = settings.latentDimension
            let latentMask = (0..<length).map { $0 < latentLength ? Float(1) : 0 }

            let textEmbedding = try await parts.encoder.run(
                [
                    "text_ids": .int(textIDs, [1, width]), "style_ttl": .float(style.ttl, [1, 50, 256]),
                    "text_mask": .float(textMask, [1, 1, width]),
                ], output: "text_emb")

            var generator = SystemRandomNumberGenerator()
            var latent = (0..<dimension * length).map { index in
                Self.gaussian(&generator) * latentMask[index % length]
            }
            for step in 0..<steps {
                guard !Task.isCancelled else { return [] }
                latent = try await parts.estimator.run(
                    [
                        "noisy_latent": .float(latent, [1, dimension, length]),
                        "text_emb": .float(textEmbedding, [1, 256, width]),
                        "style_ttl": .float(style.ttl, [1, 50, 256]),
                        "latent_mask": .float(latentMask, [1, 1, length]),
                        "text_mask": .float(textMask, [1, 1, width]),
                        "current_step": .float([Float(step)], [1]), "total_step": .float([Float(steps)], [1]),
                    ], output: "denoised_latent")
            }
            let wav = try await parts.vocoder.run(["latent": .float(latent, [1, dimension, length])], output: "wav")
            return Array(wav.prefix(min(samples, latentLength * frame)))
        } catch let failure as Part.Failure {
            throw .synthesisFailed(model: name, reason: "\(failure.part): \(failure.reason)")
        } catch {
            throw .synthesisFailed(model: name, reason: "\(error)")
        }
    }

    /// A sample of the standard normal distribution, by Box–Muller.
    private static func gaussian(_ generator: inout SystemRandomNumberGenerator) -> Float {
        let radius = Float.random(in: Float.ulpOfOne..<1, using: &generator)
        let angle = Float.random(in: 0..<1, using: &generator)
        return (-2 * log(radius)).squareRoot() * cos(2 * .pi * angle)
    }

    private func load() async throws(SpeechError) -> Loaded {
        if let loaded { return loaded }
        let name = model.spec.name
        let settings = try Settings(file: file("tts.json"), model: name)
        let indexer = try Self.indexer(file("unicode_indexer.json"), model: name)
        let cache = URL.cachesDirectory.appending(
            path: "Voices/\(model.id.rawValue)-\(model.spec.source?.commit ?? "local")", directoryHint: .isDirectory)
        let loaded = Loaded(
            settings: settings, indexer: indexer,
            duration: try await Part.load("DurationPredictor", from: model.directory, cache: cache, model: name),
            encoder: try await Part.load("TextEncoder", from: model.directory, cache: cache, model: name),
            estimator: try await Part.load("VectorEstimator", from: model.directory, cache: cache, model: name),
            vocoder: try await Part.load("Vocoder", from: model.directory, cache: cache, model: name))
        self.loaded = loaded
        return loaded
    }

    private func file(_ path: String) -> URL {
        model.directory.appending(path: path, directoryHint: .notDirectory)
    }

    private static func indexer(_ file: URL, model: String) throws(SpeechError) -> [Int32] {
        do {
            let table = try JSONDecoder().decode([Int32].self, from: Data(contentsOf: file))
            guard !table.isEmpty else {
                throw SpeechError.modelDamaged(model: model, file: file.lastPathComponent, reason: "it's empty")
            }
            return table
        } catch let error as SpeechError {
            throw error
        } catch {
            throw .modelDamaged(model: model, file: file.lastPathComponent, reason: "\(error)")
        }
    }
}

/// What a loaded Supertonic needs to speak.
private struct Loaded: Sendable {
    var settings: Settings
    var indexer: [Int32]
    var duration: Part
    var encoder: Part
    var estimator: Part
    var vocoder: Part
    /// The voices read so far, by name.
    let styles = StyleCache()

    func style(_ voice: String, of model: InstalledModel) throws(SpeechError) -> Style {
        if let style = styles.style(voice) { return style }
        let file = model.directory.appending(path: "voice_styles/\(voice).json", directoryHint: .notDirectory)
        let style = try Style(file: file, model: model.spec.name)
        styles.keep(style, as: voice)
        return style
    }
}

private final class StyleCache: Sendable {
    private let styles = Locked<[String: Style]>([:])
    func style(_ voice: String) -> Style? { styles.value[voice] }
    func keep(_ style: Style, as voice: String) { styles.withValue { $0[voice] = style } }
}

/// The model's settings, from tts.json.
private struct Settings: Sendable {
    var sampleRate: Double
    /// The samples one latent frame makes.
    var chunkSize: Int
    var latentDimension: Int

    init(file: URL, model: String) throws(SpeechError) {
        let damaged = { (reason: String) in
            SpeechError.modelDamaged(model: model, file: file.lastPathComponent, reason: reason)
        }
        let json: Any
        do {
            json = try JSONSerialization.jsonObject(with: Data(contentsOf: file))
        } catch {
            throw damaged("\(error)")
        }
        let root = json as? [String: Any]
        let ae = root?["ae"] as? [String: Any]
        let ttl = root?["ttl"] as? [String: Any]
        guard let rate = (ae?["sample_rate"] as? NSNumber)?.doubleValue,
            let base = (ae?["base_chunk_size"] as? NSNumber)?.intValue,
            let factor = (ttl?["chunk_compress_factor"] as? NSNumber)?.intValue,
            let dimension = (ttl?["latent_dim"] as? NSNumber)?.intValue,
            rate > 0, base > 0, factor > 0, dimension > 0
        else { throw damaged("its sample rate, chunk size or latent size is missing") }
        sampleRate = rate
        chunkSize = base * factor
        latentDimension = dimension * factor
    }
}

/// A voice: the styles the text encoder and the duration predictor take.
private struct Style: Sendable {
    var ttl: [Float]
    var dp: [Float]

    init(file: URL, model: String) throws(SpeechError) {
        let damaged = { (reason: String) in
            SpeechError.modelDamaged(model: model, file: "voice_styles/\(file.lastPathComponent)", reason: reason)
        }
        let json: Any
        do {
            json = try JSONSerialization.jsonObject(with: Data(contentsOf: file))
        } catch {
            throw damaged("\(error)")
        }
        let root = json as? [String: Any]
        let ttl = Self.flattened((root?["style_ttl"] as? [String: Any])?["data"])
        let dp = Self.flattened((root?["style_dp"] as? [String: Any])?["data"])
        guard ttl.count == 50 * 256, dp.count == 8 * 16 else {
            throw damaged("its styles have \(ttl.count) and \(dp.count) values, not \(50 * 256) and \(8 * 16)")
        }
        self.ttl = ttl
        self.dp = dp
    }

    private static func flattened(_ value: Any?) -> [Float] {
        if let number = value as? NSNumber { return [number.floatValue] }
        if let list = value as? [Any] { return list.flatMap { flattened($0) } }
        return []
    }
}

/// One of the model's Core ML packages, compiled and loaded.
private struct Part: @unchecked Sendable {
    struct Failure: Error {
        var part: String
        var reason: String
    }

    enum Input {
        case float([Float], [Int])
        case int([Int32], [Int])
    }

    let name: String
    // MLModel's predictions are safe to make from several threads.
    let model: MLModel

    static func load(_ name: String, from directory: URL, cache: URL, model: String) async throws(SpeechError) -> Part {
        let compiled = cache.appending(path: "\(name).mlmodelc", directoryHint: .isDirectory)
        let configuration = MLModelConfiguration()
        configuration.computeUnits = .all
        if let loaded = try? MLModel(contentsOf: compiled, configuration: configuration) {
            return Part(name: name, model: loaded)
        }
        let package = directory.appending(path: "\(name).mlpackage", directoryHint: .isDirectory)
        guard FileManager.default.fileExists(atPath: package.path) else {
            throw .modelDamaged(model: model, file: "\(name).mlpackage", reason: "it isn't there")
        }
        do {
            let temporary = try await MLModel.compileModel(at: package)
            try FileManager.default.createDirectory(at: cache, withIntermediateDirectories: true)
            try? FileManager.default.removeItem(at: compiled)
            try FileManager.default.moveItem(at: temporary, to: compiled)
            return Part(name: name, model: try MLModel(contentsOf: compiled, configuration: configuration))
        } catch {
            throw .modelUnloadable(model: model, part: name, reason: error.localizedDescription)
        }
    }

    /// Runs the part on `inputs`, each made in the type the part takes,
    /// and gives back `output`'s values as floats.
    func run(_ inputs: [String: Input], output: String) async throws -> [Float] {
        var features: [String: MLFeatureValue] = [:]
        for (key, input) in inputs {
            features[key] = MLFeatureValue(multiArray: try array(input, for: key))
        }
        let provider = try MLDictionaryFeatureProvider(dictionary: features)
        let result = try await model.prediction(from: provider)
        guard let values = result.featureValue(for: output)?.multiArrayValue else {
            throw Failure(part: name, reason: "it gave no \(output)")
        }
        return MLShapedArray<Float>(converting: values).scalars
    }

    private func array(_ input: Input, for key: String) throws -> MLMultiArray {
        let wanted = model.modelDescription.inputDescriptionsByName[key]?.multiArrayConstraint?.dataType
        switch (input, wanted) {
        case (.int(let values, let shape), .int32?), (.int(let values, let shape), nil):
            return MLMultiArray(MLShapedArray<Int32>(scalars: values, shape: shape))
        case (.int(let values, let shape), _):
            return try array(.float(values.map(Float.init), shape), for: key)
        case (.float(let values, let shape), .int32?):
            return MLMultiArray(MLShapedArray<Int32>(scalars: values.map { Int32($0) }, shape: shape))
        case (.float(let values, let shape), .double?):
            return MLMultiArray(MLShapedArray<Double>(scalars: values.map(Double.init), shape: shape))
        case (.float(let values, let shape), .float32?), (.float(let values, let shape), nil):
            return MLMultiArray(MLShapedArray<Float>(scalars: values, shape: shape))
        case (.float, let other?):
            throw Failure(part: name, reason: "\(key) takes data type \(other.rawValue), which isn't supported")
        }
    }
}
