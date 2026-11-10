import ComposableArchitecture
import InferenceKit
import InferenceKitLiteRT
import LifrAPI
import LifrConversations
import LifrModelsClient
import XCTest

/// Gemma 4 E2B on this Mac's GPU at each Thinking level: off, it answers
/// without reasoning, and on, it stops reasoning near its budget.
@MainActor
final class ThinkingOnRealModelTests: XCTestCase {
    private let question = """
        Plan a project to get back into the gym: a few tasks, each with sub-tasks, and say which day each is for.
        """

    func testOffAnswersWithoutReasoning() async throws {
        let (answer, chunks) = try await ask(thinkingTokens: nil)
        print("Off: \(chunks) chunks of reasoning, then \(answer.count) characters")
        XCTAssertEqual(chunks, 0)
        XCTAssertFalse(answer.isEmpty)
    }

    func testOnStopsReasoningNearItsBudget() async throws {
        let budget = 128
        let (answer, chunks) = try await ask(thinkingTokens: budget)
        print("On (\(budget)): \(chunks) chunks of reasoning, then \(answer.count) characters")
        XCTAssertLessThanOrEqual(chunks, budget + 16)
        XCTAssertFalse(answer.isEmpty)
    }

    /// The answer, and how many times the reasoning grew: once a token.
    private func ask(thinkingTokens: Int?) async throws -> (String, Int) {
        let model = try RealModel.installedModel()
        let engine = try await LiteRTLMEngine.load(model, on: .gpu, greedy: true)
        let chat = try await engine.chat(instructions: "You plan projects.", constrained: false)
        let chunks = LockIsolated(0)
        let answer = try await chat.send(
            question, attachments: [], format: nil, thinkingTokens: thinkingTokens,
            thought: { thinking in if thinking.duration == nil { chunks.withValue { $0 += 1 } } },
            speed: nil, partial: { _ in })
        return (answer, chunks.value)
    }
}

/// A request for a project, as the person makes it, at each Thinking
/// level: how long the whole reply takes and how much the model thought.
@MainActor
final class ThinkingLevelsOnRealModelTests: XCTestCase {
    func testAProjectAtEachLevel() async throws {
        for level in [ThinkingLevel.off, .on] {
            let model = try RealModel()
            let id = try await model.chat()
            var conversation = try XCTUnwrap(model.chatState?.conversation)
            conversation.model.thinking = level
            let chosen = ChatFeature.Action.binding(.set(\.conversation, conversation))
            model.store.send(.conversations(.chats(.element(id: id, action: chosen))))
            let clock = ContinuousClock()
            let start = clock.now
            try await model.send("Make a project for getting back into the gym, with a few tasks for this week")
            let reply = try XCTUnwrap(model.chatState?.conversation.messages.last)
            print(
                """
                LEVEL \(level): \(clock.now - start), thought \(reply.thinking?.text.count ?? 0) characters, \
                draft \(model.draft?.tasks.count ?? 0) tasks
                """)
            if level == .off { XCTAssertNil(reply.thinking) }
        }
    }
}
