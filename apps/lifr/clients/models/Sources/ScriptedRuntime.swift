import ComposableArchitecture
import Foundation
import LifrAPI

/// A model that answers each turn with the next step of a script, for tests
/// and previews. It keeps the turns it got.
package final class ScriptedRuntime: ModelRuntime {
    /// One answer: what the model does with the turn. It can call the turn's
    /// tools, report partial answers, and return or throw.
    package typealias Step =
        @Sendable (_ turn: ModelTurn, _ partial: @Sendable (JSONValue) -> Void) async throws(ModelRuntimeError) ->
        JSONValue

    package let status: ModelStatus
    package let budget: ContextBudget
    package let inputs: Set<ModelInput>
    private let steps: LockIsolated<[Step]>
    private let received = LockIsolated<[ModelTurn]>([])
    private let prewarms = LockIsolated(0)

    package init(
        status: ModelStatus = .available, budget: ContextBudget = .appleOnDevice, inputs: Set<ModelInput> = [],
        steps: [Step]
    ) {
        self.status = status
        self.budget = budget
        self.inputs = inputs
        self.steps = LockIsolated(steps)
    }

    /// The turns the model got, in order.
    package var turns: [ModelTurn] { received.value }
    /// How many times it was asked to prewarm.
    package var prewarmCount: Int { prewarms.value }

    package func prewarm() { prewarms.withValue { $0 += 1 } }

    package func respond(to turn: ModelTurn, partial: @escaping @Sendable (JSONValue) -> Void)
        async throws(ModelRuntimeError) -> JSONValue
    {
        received.withValue { $0.append(turn) }
        guard status == .available else { throw .unavailable(status) }
        if let kind = turn.unreadable(by: inputs) { throw .unsupportedInput(kind) }
        let step = steps.withValue { steps in steps.isEmpty ? nil : steps.removeFirst() }
        guard let step else { throw .failed("the script has no more answers") }
        return try await step(turn, partial)
    }
}

extension ScriptedRuntime {
    /// Answers with `answer`, after reporting each of `partials`.
    package static func answer(_ answer: JSONValue, partials: [JSONValue] = []) -> Step {
        { _, partial throws(ModelRuntimeError) in
            for value in partials { partial(value) }
            partial(answer)
            return answer
        }
    }

    /// Thinks `text` for `duration` on `processor` when the turn asks for
    /// thinking, then goes on with `then`.
    package static func think(
        _ text: String, for duration: Duration, on processor: ModelBackend = .gpu, then: @escaping Step
    ) -> Step {
        { turn, partial throws(ModelRuntimeError) in
            turn.thinking?(ModelThinking(text: text, processor: processor))
            let answer = try await then(turn, partial)
            turn.thinking?(ModelThinking(text: text, duration: duration, processor: processor))
            return answer
        }
    }

    /// Fails with `error`.
    package static func fail(_ error: ModelRuntimeError) -> Step {
        { _, _ throws(ModelRuntimeError) in throw error }
    }

    /// Calls the turn's tool `name` with `arguments`, then goes on with
    /// `then`. Fails when the turn doesn't have the tool.
    package static func call(_ name: String, _ arguments: JSONValue, then: @escaping Step) -> Step {
        { turn, partial throws(ModelRuntimeError) in
            guard let tool = turn.tools.first(where: { $0.name == name }) else {
                throw .failed("the turn has no tool \(name)")
            }
            _ = await tool.call(arguments)
            return try await then(turn, partial)
        }
    }
}
