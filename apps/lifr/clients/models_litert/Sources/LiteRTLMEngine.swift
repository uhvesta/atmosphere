import Foundation
import LifrAPI
import LifrModelsClient
import LifrModelsLive
import LiteRTLM

/// LiteRT-LM's engine for one downloaded model, on the GPU through Metal.
package struct LiteRTLMEngine: LiteRTEngine {
    let engine: Engine
    /// Whether each turn takes the likeliest next token, rather than the
    /// model's own sampling.
    let greedy: Bool

    /// Loads `model`'s `.litertlm` file. LiteRT-LM keeps the GPU's compiled
    /// programs in Caches, so the next load is quicker.
    @Sendable package static func load(_ model: InstalledModel) async throws(ModelRuntimeError) -> any LiteRTEngine {
        try await load(model, greedy: false)
    }

    /// Loads `model`, taking the likeliest token at each step with
    /// `greedy`, so a turn reads the same each time on one Mac. The GPU's
    /// arithmetic still differs between machines.
    package static func load(_ model: InstalledModel, greedy: Bool) async throws(ModelRuntimeError)
        -> any LiteRTEngine
    {
        guard let file = model.spec.source?.files.first(where: { $0.path.hasSuffix(".litertlm") }) else {
            throw .unavailable(.notDownloaded)
        }
        let path = model.location(of: file).path
        guard FileManager.default.fileExists(atPath: path) else { throw .unavailable(.notDownloaded) }
        do {
            let cache = try cacheDirectory(model.id)
            // Gemma's vision encoder runs on the GPU and its audio encoder
            // on the CPU, as Google runs them; each loads only for a model
            // that reads photos or recordings.
            let config = try EngineConfig(
                modelPath: path, backend: .gpu,
                visionBackend: model.spec.inputs.contains(.image) ? .gpu : nil,
                audioBackend: model.spec.inputs.contains(.audio) ? .cpu() : nil,
                maxNumTokens: model.spec.contextTokens, cacheDir: cache.path)
            let engine = Engine(engineConfig: config)
            try await engine.initialize()
            return LiteRTLMEngine(engine: engine, greedy: greedy)
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
            return LiteRTLMChat(conversation: conversation)
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
    let conversation: Conversation

    func send(
        _ message: String, attachments: [ModelAttachment], format: String?,
        partial: @escaping @Sendable (String) -> Void
    ) async throws(ModelRuntimeError) -> String {
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
            var written = ""
            for try await chunk in conversation.sendMessageStream(
                Message(contents: contents), responseFormat: responseFormat)
            {
                written += chunk.toString
                partial(written)
            }
            if Task.isCancelled { throw ModelRuntimeError.cancelled }
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
