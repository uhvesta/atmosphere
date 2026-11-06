import Foundation
import LifrAPI
import LifrModelsClient
import LifrModelsLive
import LiteRTLM

/// LiteRT-LM's engine for one downloaded model, on the GPU through Metal,
/// or on the CPU while Lifr answers in the background.
package struct LiteRTLMEngine: LiteRTEngine {
    let engine: Engine
    /// Whether each turn takes the likeliest next token, rather than the
    /// model's own sampling.
    let greedy: Bool
    /// What it runs on.
    var backend: ModelBackend = .gpu

    /// Loads `model`'s `.litertlm` file to run on `backend`. LiteRT-LM keeps
    /// the GPU's compiled programs in Caches, so the next load is quicker.
    @Sendable package static func load(_ model: InstalledModel, on backend: ModelBackend)
        async throws(ModelRuntimeError) -> any LiteRTEngine
    {
        try await load(model, on: backend, greedy: false)
    }

    /// Loads `model`, taking the likeliest token at each step with
    /// `greedy`, so a turn reads the same each time on one Mac. The GPU's
    /// arithmetic still differs between machines.
    package static func load(_ model: InstalledModel, on backend: ModelBackend = .gpu, greedy: Bool)
        async throws(ModelRuntimeError) -> any LiteRTEngine
    {
        guard let file = model.spec.source?.files.first(where: { $0.path.hasSuffix(".litertlm") }) else {
            throw .unavailable(.notDownloaded)
        }
        let path = model.location(of: file).path
        guard FileManager.default.fileExists(atPath: path) else { throw .unavailable(.notDownloaded) }
        do {
            let cache = try cacheDirectory(model.id)
            // Gemma's vision encoder runs where the model does and its
            // audio encoder on the CPU, as Google runs them; each loads
            // only for a model that reads photos or recordings.
            let main: Backend =
                switch backend {
                case .gpu: .gpu
                case .cpu: .cpu()
                }
            let config = try EngineConfig(
                modelPath: path, backend: main,
                visionBackend: model.spec.inputs.contains(.image) ? main : nil,
                audioBackend: model.spec.inputs.contains(.audio) ? .cpu() : nil,
                maxNumTokens: model.spec.contextTokens, cacheDir: cache.path,
                // On the GPU, Gemma 4's half-precision activations garble
                // digits once a prompt is past about 2,000 tokens, writing
                // a date such as 2026-11-06 as 20026-111-06 (LiteRT-LM
                // issue 2814). Full precision writes them as the CPU does.
                activationDataType: backend == .gpu ? .float32 : nil)
            let engine = Engine(engineConfig: config)
            try await engine.initialize()
            return LiteRTLMEngine(engine: engine, greedy: greedy, backend: backend)
        } catch {
            throw runtimeError(error)
        }
    }

    package func chat(instructions: String, constrained: Bool) async throws(ModelRuntimeError) -> any LiteRTChat {
        do {
            let sampler = greedy ? try SamplerConfig(topK: 1, topP: 1, temperature: 0) : nil
            let conversation = try await engine.createConversation(
                with: ConversationConfig(
                    systemMessage: Message(instructions, role: .system), samplerConfig: sampler,
                    automaticToolCalling: false, enableResponseFormat: constrained))
            return LiteRTLMChat(conversation: conversation, backend: backend)
        } catch {
            throw runtimeError(error)
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
