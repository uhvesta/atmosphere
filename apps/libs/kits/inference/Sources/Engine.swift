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
    /// answer as far as it's written, each time it grows. With `thinks`,
    /// the model reasons before it answers. `thought` gets that reasoning
    /// as it grows, then how long the answer took.
    func send(
        _ message: String, attachments: [ModelAttachment], format: String?, thinks: Bool,
        thought: (@Sendable (ModelThinking) -> Void)?, partial: @escaping @Sendable (String) -> Void
    ) async throws(ModelRuntimeError) -> String
    /// Stops the answer being written.
    func cancel()
}
