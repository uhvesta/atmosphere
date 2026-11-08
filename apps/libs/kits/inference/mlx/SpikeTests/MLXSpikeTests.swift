import Foundation
import MLXLLM
import MLXLMCommon
import Tokenizers
import XCTest

/// Qwen3.5 0.8B through MLX on this Mac's GPU: it answers, and how fast.
final class MLXSpikeTests: XCTestCase {
    func testItAnswersAndMeasuresItsSpeed() async throws {
        let model = try await LLMModelFactory.shared.loadContainer(
            from: try Self.modelDirectory(), using: TransformersLoader())

        // The first answer also compiles the kernels MLX builds on first use.
        let (answer, first) = try await Self.respond(
            "What is the capital of France?", on: model, instructions: "Answer in one short sentence.")
        XCTAssertTrue(answer.localizedCaseInsensitiveContains("paris"), answer)
        print("MLX spike, first answer: \(first.summary())")

        let (_, warm) = try await Self.respond(
            "Write three paragraphs about the sea.", on: model, instructions: "", maxTokens: 400)
        print("MLX spike, warm: \(warm.summary())")
        XCTAssertGreaterThan(warm.tokensPerSecond, 0)
    }

    private static func respond(
        _ prompt: String, on model: ModelContainer, instructions: String, maxTokens: Int = 64
    ) async throws -> (String, GenerateCompletionInfo) {
        let session = ChatSession(
            model, instructions: instructions,
            generateParameters: GenerateParameters(maxTokens: maxTokens, temperature: 0),
            additionalContext: ["enable_thinking": false])
        var answer = ""
        var info: GenerateCompletionInfo?
        for try await generation in session.streamDetails(to: prompt) {
            switch generation {
            case .chunk(let text): answer += text
            case .info(let completion): info = completion
            default: break
            }
        }
        return (answer, try XCTUnwrap(info))
    }

    /// The model Bazel downloaded.
    static func modelDirectory() throws -> URL {
        let runfiles = try XCTUnwrap(
            ProcessInfo.processInfo.environment["TEST_SRCDIR"],
            "Run this with bazel test, which says where the model is.")
        return URL(fileURLWithPath: runfiles).appending(path: TestModel.config).deletingLastPathComponent()
    }
}

/// Loads a model's tokenizer with swift-transformers.
private struct TransformersLoader: MLXLMCommon.TokenizerLoader {
    func load(from directory: URL) async throws -> any MLXLMCommon.Tokenizer {
        TokenizerBridge(try await AutoTokenizer.from(modelFolder: directory))
    }
}

/// swift-transformers' tokenizer as MLX's.
private struct TokenizerBridge: MLXLMCommon.Tokenizer {
    let upstream: any Tokenizers.Tokenizer

    init(_ upstream: any Tokenizers.Tokenizer) {
        self.upstream = upstream
    }

    func encode(text: String, addSpecialTokens: Bool) -> [Int] {
        upstream.encode(text: text, addSpecialTokens: addSpecialTokens)
    }

    func decode(tokenIds: [Int], skipSpecialTokens: Bool) -> String {
        upstream.decode(tokens: tokenIds, skipSpecialTokens: skipSpecialTokens)
    }

    func convertTokenToId(_ token: String) -> Int? { upstream.convertTokenToId(token) }
    func convertIdToToken(_ id: Int) -> String? { upstream.convertIdToToken(id) }
    var bosToken: String? { upstream.bosToken }
    var eosToken: String? { upstream.eosToken }
    var unknownToken: String? { upstream.unknownToken }

    func applyChatTemplate(
        messages: [[String: any Sendable]], tools: [[String: any Sendable]]?, additionalContext: [String: any Sendable]?
    ) throws -> [Int] {
        do {
            return try upstream.applyChatTemplate(
                messages: messages, tools: tools, additionalContext: additionalContext)
        } catch Tokenizers.TokenizerError.missingChatTemplate {
            throw MLXLMCommon.TokenizerError.missingChatTemplate
        }
    }
}
