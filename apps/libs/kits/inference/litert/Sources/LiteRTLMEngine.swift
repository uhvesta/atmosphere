import Foundation
import InferenceKit
import InferenceKitLive
import JSONKit
import LiteRTLM

/// LiteRT-LM's engine for one downloaded model, on the GPU through Metal,
/// or on the CPU while the app answers in the background.
public struct LiteRTLMEngine: LiteRTEngine {
    let engine: Engine
    /// How each turn picks its tokens; nil for the model's own way.
    let sampling: ModelProfile.Sampling?
    /// What it runs on.
    var backend: ModelBackend = .gpu

    /// Loads `model`'s `.litertlm` file to run on `backend`, as its
    /// profile says it runs best. LiteRT-LM keeps the GPU's compiled
    /// programs in Caches, so the next load is quicker.
    @Sendable public static func load(_ model: InstalledModel, on backend: ModelBackend)
        async throws(ModelRuntimeError) -> any LiteRTEngine
    {
        try await load(model, on: backend, profile: model.spec.profile)
    }

    /// Loads `model`, taking the likeliest token at each step with
    /// `greedy`, so a turn reads the same each time on one Mac. The GPU's
    /// arithmetic still differs between machines.
    public static func load(_ model: InstalledModel, on backend: ModelBackend = .gpu, greedy: Bool)
        async throws(ModelRuntimeError) -> any LiteRTEngine
    {
        var profile = model.spec.profile
        if greedy { profile.sampling = .greedy }
        return try await load(model, on: backend, profile: profile)
    }

    /// Loads `model` to run on `backend` as `profile` says, such as to
    /// measure one profile against another.
    public static func load(_ model: InstalledModel, on backend: ModelBackend, profile: ModelProfile)
        async throws(ModelRuntimeError) -> any LiteRTEngine
    {
        guard let file = model.spec.source?.files.first(where: { $0.path.hasSuffix(".litertlm") }) else {
            throw .unavailable(.notDownloaded)
        }
        let path = model.location(of: file).path
        guard FileManager.default.fileExists(atPath: path) else { throw .unavailable(.notDownloaded) }
        do {
            let cache = try cacheDirectory(model.id)
            let cpu: Backend = .cpu(threadCount: profile.cpuThreads)
            let main: Backend =
                switch backend {
                case .gpu: .gpu
                case .cpu: cpu
                }
            func place(_ encoder: ModelProfile.Encoder) -> Backend { encoder == .cpu ? cpu : main }
            // Each encoder loads only for a model that reads photos or
            // listens to recordings.
            let config = try EngineConfig(
                modelPath: path, backend: main,
                visionBackend: model.spec.inputs.contains(.image) ? place(profile.vision) : nil,
                audioBackend: model.spec.inputs.contains(.audio) ? place(profile.audio) : nil,
                maxNumTokens: model.window, cacheDir: cache.path,
                activationDataType: backend == .gpu && profile.gpuActivations == .full ? .float32 : nil,
                speculativeDecodingConfig: SpeculativeDecodingConfig(mode: mode(profile.speculation)))
            let engine = Engine(engineConfig: config)
            try await engine.initialize()
            return LiteRTLMEngine(engine: engine, sampling: profile.sampling, backend: backend)
        } catch {
            throw runtimeError(error)
        }
    }

    public func chat(instructions: String, constrained: Bool) async throws(ModelRuntimeError) -> any LiteRTChat {
        do {
            let sampler = try sampling.map {
                try SamplerConfig(topK: $0.topK, topP: $0.topP, temperature: $0.temperature)
            }
            let conversation = try await engine.createConversation(
                with: ConversationConfig(
                    systemMessage: Message(instructions, role: .system), samplerConfig: sampler,
                    automaticToolCalling: false, enableResponseFormat: constrained))
            return LiteRTLMChat(conversation: conversation, backend: backend)
        } catch {
            throw runtimeError(error)
        }
    }

    private static func mode(_ speculation: ModelProfile.Speculation) -> SpeculativeDecodingMode {
        switch speculation {
        case .off: .off
        case .adaptive: .adaptive
        case .on: .on
        }
    }

    private static func cacheDirectory(_ id: ModelID) throws -> URL {
        let caches = try FileManager.default.url(
            for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        let directory = caches.appending(path: "LiteRT", directoryHint: .isDirectory)
            .appending(path: id.directoryName, directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }
}

/// One LiteRT-LM conversation.
struct LiteRTLMChat: LiteRTChat {
    /// The most a turn thinks before it answers, in tokens: about 20
    /// seconds on an iPhone 17 Pro's GPU.
    static let thinkingTokens = 512

    let conversation: Conversation
    let backend: ModelBackend

    func send(
        _ message: String, attachments: [ModelAttachment], format: String?, thinks: Bool,
        thought: (@Sendable (ModelThinking) -> Void)?, partial: @escaping @Sendable (String) -> Void
    ) async throws(ModelRuntimeError) -> String {
        let clock = ContinuousClock()
        let start = clock.now
        do {
            let responseFormat = try format.map { try ResponseFormat.json(schema: $0) }
            // Gemma reads photos and recordings before the words about them.
            let contents =
                attachments.map { attachment -> Content in
                    switch attachment.kind {
                    case .image: .imageFile(attachment.file.path)
                    case .audio: .audioFile(attachment.file.path)
                    }
                } + [.text(message)]
            // Off unless asked for: Gemma 4 can think by default.
            let thinking = ThinkingConfig(enableThinking: thinks, thinkingTokenBudget: Self.thinkingTokens)
            var written = ""
            var reasoning = ""
            for try await chunk in conversation.sendMessageStream(
                Message(contents: contents), thinkingConfig: thinking, responseFormat: responseFormat)
            {
                // Gemma writes its reasoning to a channel of its own.
                let thinkingPart = chunk.channels.values.joined()
                if !thinkingPart.isEmpty {
                    reasoning += thinkingPart
                    thought?(ModelThinking(text: reasoning, processor: backend))
                }
                let answerPart = chunk.toString
                if !answerPart.isEmpty {
                    written += answerPart
                    partial(written)
                }
            }
            if Task.isCancelled { throw ModelRuntimeError.cancelled }
            thought?(ModelThinking(text: reasoning, duration: clock.now - start, processor: backend))
            return written
        } catch {
            throw runtimeError(error)
        }
    }

    func cancel() {
        try? conversation.cancel()
    }
}

/// What a LiteRT-LM error means for the turn.
private func runtimeError(_ error: any Error) -> ModelRuntimeError {
    if let error = error as? ModelRuntimeError { return error }
    if error is CancellationError || Task.isCancelled { return .cancelled }
    let detail = error.localizedDescription
    let lowered = detail.lowercased()
    if lowered.contains("token"), lowered.contains("exceed") || lowered.contains("max") || lowered.contains("limit") {
        return .exceededContext
    }
    return .failed(detail)
}
