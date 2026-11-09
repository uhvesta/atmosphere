/// A downloaded model, loaded by one of the engines an app is built with,
/// such as LiteRT-LM. Each turn gets a conversation of its own.
public protocol ModelEngine: Sendable {
    /// A new conversation that follows `instructions`. `constrained` when
    /// its answers will be held to a JSON schema.
    func chat(instructions: String, constrained: Bool) async throws(ModelRuntimeError) -> any EngineChat
}

/// One conversation with a loaded model.
public protocol EngineChat: Sendable {
    /// What the model writes back to `message` and its `attachments`, held
    /// to the JSON schema `format` when there is one. `partial` gets the
    /// answer as far as it's written, each time it grows. With
    /// `thinkingTokens`, the model reasons for at most that many tokens
    /// before it answers. `thought` gets that reasoning as it grows, then
    /// how long the answer took. `speed` gets how many tokens the model
    /// read and wrote, and how long each took, once it's done.
    func send(
        _ message: String, attachments: [ModelAttachment], format: String?, thinkingTokens: Int?,
        thought: (@Sendable (ModelThinking) -> Void)?, speed: (@Sendable (TurnSpeed) -> Void)?,
        partial: @escaping @Sendable (String) -> Void
    ) async throws(ModelRuntimeError) -> String
    /// Stops the answer being written.
    func cancel()
}
