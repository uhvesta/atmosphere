import Foundation
import InferenceKit

/// MLX's engine for one downloaded model in MLX's format, on the GPU
/// through Metal. It doesn't run on the CPU: some models' layers, such as
/// Qwen3.5's linear attention, are Metal kernels of MLX's own, and as
/// plain operations on the CPU they run hundreds of times slower.
/// It writes the model's prompt itself, in the ChatML the catalog's MLX
/// models read.
public struct MLXEngine: ModelEngine {
    let session: MLXSession

    /// Loads `model`'s folder to run on `backend`, as its profile says it
    /// runs best.
    @Sendable public static func load(_ model: InstalledModel, on backend: ModelBackend)
        async throws(ModelRuntimeError) -> any ModelEngine
    {
        try await load(model, on: backend, profile: model.spec.profile)
    }

    /// Loads `model`, taking the likeliest token at each step with
    /// `greedy`, so a turn reads the same each time on one Mac.
    public static func load(_ model: InstalledModel, on backend: ModelBackend = .gpu, greedy: Bool)
        async throws(ModelRuntimeError) -> any ModelEngine
    {
        var profile = model.spec.profile
        if greedy { profile.sampling = .greedy }
        return try await load(model, on: backend, profile: profile)
    }

    /// Loads `model` to run on `backend` as `profile` says.
    public static func load(_ model: InstalledModel, on backend: ModelBackend, profile: ModelProfile)
        async throws(ModelRuntimeError) -> any ModelEngine
    {
        MLXEngine(session: try await open(model, on: backend, profile: profile))
    }

    /// Reads a fixed prompt of 256 tokens on the GPU and writes 128,
    /// taking the likeliest token each time, at `model`'s window.
    @Sendable public static func measure(_ model: InstalledModel, on backend: ModelBackend)
        async throws(ModelRuntimeError) -> EngineSpeed
    {
        var profile = model.spec.profile
        profile.sampling = .greedy
        let session = try await open(model, on: backend, profile: profile)
        return try await session.run(stoppedBy: MLXStop()) { session throws(ModelRuntimeError) in
            try session.benchmark(prompt: "Plan a short week of tasks, one line each.", prefill: 256, decode: 128)
        }
    }

    public func chat(instructions: String, constrained: Bool) async throws(ModelRuntimeError) -> any EngineChat {
        MLXChat(session: session, instructions: instructions)
    }

    private static func open(_ model: InstalledModel, on backend: ModelBackend, profile: ModelProfile)
        async throws(ModelRuntimeError) -> MLXSession
    {
        guard backend == .gpu else {
            throw .failed(
                "MLX runs models only on the GPU, which apps get only on screen. Open the app to have it answer.")
        }
        let files = model.spec.source?.files ?? []
        guard let config = files.first(where: { $0.path == "config.json" }) else {
            throw .unavailable(.notDownloaded)
        }
        for file in files where !FileManager.default.fileExists(atPath: model.location(of: file).path) {
            throw .unavailable(.notDownloaded)
        }
        return try await MLXSession.open(
            directory: model.location(of: config).deletingLastPathComponent(), backend: backend,
            window: model.window, profile: profile)
    }
}

/// One conversation with an MLX model. Its history stays in the session
/// until it's let go of.
final class MLXChat: EngineChat {
    /// How a turn picks its tokens when the model's profile doesn't say.
    static let sampling = ModelProfile.Sampling(topK: 40, topP: 0.95, temperature: 0.7)

    private let session: MLXSession
    private let instructions: String
    private let id = UUID()
    private let stop = MLXStop()

    init(session: MLXSession, instructions: String) {
        self.session = session
        self.instructions = instructions
    }

    deinit {
        session.forget(id)
    }

    func send(
        _ message: String, attachments: [ModelAttachment], format: String?, thinkingTokens: Int?,
        thought: (@Sendable (ModelThinking) -> Void)?, partial: @escaping @Sendable (String) -> Void
    ) async throws(ModelRuntimeError) -> String {
        if let attachment = attachments.first { throw .unsupportedInput(attachment.kind) }
        let start = ContinuousClock.now
        let turn = Turn(
            id: id, instructions: instructions, message: message, schema: format, thinkingTokens: thinkingTokens,
            stop: stop, thought: thought, partial: partial)
        let (answer, reasoning) = try await session.run(stoppedBy: stop) { session throws(ModelRuntimeError) in
            try turn.run(on: session)
        }
        if let reasoning {
            thought?(ModelThinking(text: reasoning, duration: .now - start, processor: session.backend))
        }
        return answer
    }

    func cancel() {
        stop.stop()
    }
}

/// One message and its answer, as it runs on the session's queue.
private struct Turn: Sendable {
    var id: UUID
    var instructions: String
    var message: String
    /// The JSON Schema the answer keeps to, if it has one.
    var schema: String?
    /// The most the model thinks for, or nil when it answers straight away.
    var thinkingTokens: Int?
    var stop: MLXStop
    var thought: (@Sendable (ModelThinking) -> Void)?
    var partial: @Sendable (String) -> Void

    /// Whether the turn asked the model to think.
    private var thinks: Bool { thinkingTokens != nil }

    /// The answer, and the reasoning to show for it, if the model showed
    /// any.
    func run(on session: MLXSession) throws(ModelRuntimeError) -> (String, String?) {
        try prompt(on: session)
        let thinking = session.profile.thinking
        let sampling = session.profile.sampling ?? MLXChat.sampling
        // A model that always thinks does, but its reasoning is shown only
        // when the turn asked for it.
        var shown: String?
        if let thinking, thinks || thinking.off == nil {
            let reasoning = try think(on: session, thinking: thinking, sampling: sampling)
            guard let reasoning else { return ("", thinks ? "" : nil) }
            shown = thinks ? reasoning : nil
        }
        let answer = try answer(on: session, sampler: session.sampler(schema: schema, sampling: sampling))
        return (answer, shown ?? (thinks ? "" : nil))
    }

    /// Gives the model the message, after the instructions if it's the
    /// first, and starts its answer.
    private func prompt(on session: MLXSession) throws(ModelRuntimeError) {
        if session.isNew(id) {
            if instructions.isEmpty {
                try session.give("<|im_start|>user\n", special: true, in: id)
            } else {
                try session.give("<|im_start|>system\n", special: true, in: id)
                try session.give(instructions, special: false, in: id)
                try session.give("<|im_end|>\n<|im_start|>user\n", special: true, in: id)
            }
        } else {
            // The model's last answer ended where it wrote its end token,
            // which isn't read back.
            try session.give("<|im_end|>\n<|im_start|>user\n", special: true, in: id)
        }
        try session.give(message, special: false, in: id)
        var start = "<|im_end|>\n<|im_start|>assistant\n"
        if let thinking = session.profile.thinking {
            start += (thinks ? thinking.on : thinking.off) ?? thinking.on
        }
        try session.give(start, special: true, in: id)
    }

    /// The model's reasoning, until it writes `thinking.end` or reaches
    /// the budget, after which it answers; nil when it ended its turn
    /// while thinking.
    private func think(
        on session: MLXSession, thinking: ModelProfile.Thinking, sampling: ModelProfile.Sampling
    ) throws(ModelRuntimeError) -> String? {
        let sampler = try session.sampler(schema: nil, sampling: sampling)
        var tokens: [MLXSession.Token] = []
        // A model that thinks when not asked to gets the usual budget.
        for _ in 0..<(thinkingTokens ?? ThinkingLevel.on.tokens ?? 0) {
            if stop.isStopped { throw .cancelled }
            guard session.room > 8 else { throw .exceededContext }
            let (token, ends) = try session.next(sampler)
            if ends { return nil }
            try session.give([token], in: id)
            tokens.append(token)
            let text = session.text(of: tokens, special: true)
            if let end = text.range(of: thinking.end) {
                try session.give("\n\n", special: false, in: id)
                return trimmed(String(text[..<end.lowerBound]))
            }
            if thinks { thought?(ModelThinking(text: partialText(text), processor: session.backend)) }
        }
        // Out of budget: it's stopped thinking for it.
        try session.give("\n\(thinking.end)\n\n", special: true, in: id)
        return trimmed(session.text(of: tokens, special: true))
    }

    /// The model's answer, until it ends its turn.
    private func answer(on session: MLXSession, sampler: MLXSampler) throws(ModelRuntimeError) -> String {
        var tokens: [MLXSession.Token] = []
        while true {
            if stop.isStopped { throw .cancelled }
            guard session.room > 1 else { throw .exceededContext }
            let (token, ends) = try session.next(sampler)
            if ends { break }
            try session.give([token], in: id)
            tokens.append(token)
            partial(partialText(session.text(of: tokens, special: false)))
        }
        return trimmed(session.text(of: tokens, special: false))
    }

    /// Text as far as it's written, without a character cut in half.
    private func partialText(_ text: String) -> String {
        var text = trimmed(text)
        while text.last == "\u{FFFD}" { text.removeLast() }
        return text
    }

    private func trimmed(_ text: String) -> String {
        text.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
