import Foundation
import InferenceKit
import InferenceKitLlama
import XCTest

/// Qwen3.5 0.8B through llama.cpp on this Mac: it answers, keeps to a
/// schema, thinks when asked, and remembers the conversation.
final class LlamaSmokeTests: XCTestCase {
    func testItAnswersOnTheGPUAndTheCPU() async throws {
        for backend in [ModelBackend.gpu, .cpu] {
            let engine = try await LlamaEngine.load(try Self.installedModel(), on: backend, greedy: true)
            let chat = try await engine.chat(instructions: "Answer in one short sentence.", constrained: false)

            let answer = try await chat.send(
                "What is the capital of France?", attachments: [], format: nil, thinkingTokens: nil, thought: nil,
                partial: { _ in })

            XCTAssertTrue(answer.localizedCaseInsensitiveContains("paris"), "\(backend): \(answer)")
        }
    }

    func testAnAnswerKeepsToItsSchema() async throws {
        let engine = try await LlamaEngine.load(try Self.installedModel(), on: .gpu, greedy: true)
        let chat = try await engine.chat(instructions: "", constrained: true)
        let schema = #"""
            {"type":"object","properties":{"title":{"type":"string"},"minutes":{"type":"integer"}},
            "required":["title","minutes"]}
            """#

        let answer = try await chat.send(
            "A task: walk the dog for half an hour.", attachments: [], format: schema, thinkingTokens: nil,
            thought: nil,
            partial: { _ in })

        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(answer.utf8)) as? [String: Any], answer)
        XCTAssertNotNil(object["title"] as? String, answer)
        XCTAssertNotNil(object["minutes"] as? Int, answer)
    }

    func testItThinksWhenAskedAndRemembersTheConversation() async throws {
        let engine = try await LlamaEngine.load(try Self.installedModel(), on: .gpu, greedy: true)
        let chat = try await engine.chat(instructions: "", constrained: false)
        let thoughts = LockIsolated<[ModelThinking]>([])

        _ = try await chat.send(
            "My cat is called Miso. Say hello to her.", attachments: [], format: nil, thinkingTokens: 512,
            thought: { thinking in thoughts.withValue { $0.append(thinking) } }, partial: { _ in })
        let answer = try await chat.send(
            "What is my cat called?", attachments: [], format: nil, thinkingTokens: nil, thought: nil, partial: { _ in }
        )

        XCTAssertFalse(thoughts.value.isEmpty)
        XCTAssertTrue(answer.localizedCaseInsensitiveContains("miso"), answer)
    }

    func testItMeasuresItsSpeed() async throws {
        let speed = try await LlamaEngine.measure(try Self.installedModel(), on: .gpu)

        XCTAssertGreaterThan(speed.decodeTokensPerSecond, 0)
    }

    /// The model Bazel downloaded, as the catalog pins it.
    static func installedModel() throws -> InstalledModel {
        let entry = ModelCatalog.qwen35Tiny
        let gguf = try XCTUnwrap(
            entry.variants.first { $0.runtime == .llama }, "The catalog's Qwen3.5 0.8B has no GGUF")
        let spec = entry.running(gguf)
        let file = try XCTUnwrap(spec.source?.files.first, "The catalog's Qwen3.5 0.8B has no file")
        XCTAssertEqual(
            file.digest, .sha256(TestModel.sha256),
            """
            The catalog and test_model.bzl pin different files. Pin both to the same commit and SHA-256, from \
            the model's page on Hugging Face, so this tests what the app downloads.
            """)
        let runfiles = try XCTUnwrap(
            ProcessInfo.processInfo.environment["TEST_SRCDIR"],
            "Run this with bazel test, which says where the model is.")
        let path = URL(fileURLWithPath: runfiles).appending(path: TestModel.runfile)
        XCTAssertEqual(path.lastPathComponent, file.path)
        return InstalledModel(spec: spec, directory: path.deletingLastPathComponent(), installedAt: Date())
    }
}

/// A value shared between the test and the engine's callbacks.
final class LockIsolated<Value: Sendable>: @unchecked Sendable {
    private var stored: Value
    private let lock = NSLock()

    init(_ value: Value) { stored = value }

    var value: Value { lock.withLock { stored } }

    func withValue(_ change: (inout Value) -> Void) { lock.withLock { change(&stored) } }
}
