import Foundation
import LifrAPI
import LifrModelsClient
import LifrModelsLive
import LiteRTLM

/// LiteRT-LM's engine for one downloaded model, on the GPU through Metal.
package struct LiteRTLMEngine: LiteRTEngine {
    let engine: Engine

    /// Loads `model`'s `.litertlm` file. LiteRT-LM keeps the GPU's compiled
    /// programs in Caches, so the next load is quicker.
    @Sendable package static func load(_ model: InstalledModel) async throws(ModelRuntimeError) -> any LiteRTEngine {
        guard let file = model.spec.source?.files.first(where: { $0.path.hasSuffix(".litertlm") }) else {
            throw .unavailable(.notDownloaded)
        }
        let path = model.location(of: file).path
        guard FileManager.default.fileExists(atPath: path) else { throw .unavailable(.notDownloaded) }
        do {
            let cache = try cacheDirectory(model.id)
            let config = try EngineConfig(
                modelPath: path, backend: .gpu, maxNumTokens: model.spec.contextTokens, cacheDir: cache.path)
            let engine = Engine(engineConfig: config)
            try await engine.initialize()
            return LiteRTLMEngine(engine: engine)
        } catch {
            throw runtimeError(error)
        }
    }

    package func chat(instructions: String, constrained: Bool) async throws(ModelRuntimeError) -> any LiteRTChat {
        do {
            let conversation = try await engine.createConversation(
                with: ConversationConfig(
                    systemMessage: Message(instructions, role: .system), automaticToolCalling: false,
                    enableResponseFormat: constrained))
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

    func send(_ message: String, format: String?, partial: @escaping @Sendable (String) -> Void)
        async throws(ModelRuntimeError) -> String
    {
        do {
            let responseFormat = try format.map { try ResponseFormat.json(schema: $0) }
            var written = ""
            for try await chunk in conversation.sendMessageStream(Message(message), responseFormat: responseFormat) {
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
