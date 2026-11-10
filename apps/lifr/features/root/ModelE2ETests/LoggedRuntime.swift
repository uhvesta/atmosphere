import LifrAPI
import LifrModelsClient

/// A runtime that prints what the model gets and does, so a failing
/// request shows why.
struct LoggedRuntime: ModelRuntime {
    let base: any ModelRuntime

    var status: ModelStatus { base.status }
    var budget: ContextBudget { base.budget }
    var inputs: Set<ModelInput> { base.inputs }
    func prewarm() { base.prewarm() }

    func respond(to turn: ModelTurn, partial: @escaping @Sendable (JSONValue) -> Void)
        async throws(ModelRuntimeError) -> JSONValue
    {
        var turn = turn
        turn.tools = turn.tools.map { tool in
            var tool = tool
            let call = tool.call
            tool.call = { arguments in
                let result = await call(arguments)
                print("TOOL \(tool.name)(\(arguments)) → \(result)")
                return result
            }
            return tool
        }
        print("TOOLS \(turn.tools.map(\.name))\nINSTRUCTIONS\n\(turn.instructions)\nPROMPT\n\(turn.prompt)")
        let clock = ContinuousClock()
        let start = clock.now
        let answer = try await base.respond(to: turn, partial: partial)
        let thinks = turn.thinking == nil ? "no" : "up to \(turn.thinkingTokens) tokens"
        print("ANSWER after \(clock.now - start), thinking \(thinks)\n\(answer)")
        return answer
    }
}
