import Foundation
import InferenceKit
import InferenceKitGrammar

/// llama.cpp's engine for one downloaded GGUF model, on the GPU through
/// Metal, or on the CPU while the app answers in the background. It writes
/// the model's prompt itself, in the ChatML the catalog's models read.
public struct LlamaEngine: ModelEngine {
    let session: LlamaSession

    /// Loads `model`'s `.gguf` file to run on `backend`, as its profile
    /// says it runs best.
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
        LlamaEngine(session: try await open(model, on: backend, profile: profile))
    }

    /// Reads a fixed prompt of 256 tokens on `backend` and writes 128,
    /// taking the likeliest token each time, at `model`'s window.
    @Sendable public static func measure(_ model: InstalledModel, on backend: ModelBackend)
        async throws(ModelRuntimeError) -> EngineSpeed
    {
        var profile = model.spec.profile
        profile.sampling = .greedy
        let session = try await open(model, on: backend, profile: profile)
        return try await session.run(stoppedBy: LlamaStop()) { session throws(ModelRuntimeError) in
            try session.benchmark(prompt: "Plan a short week of tasks, one line each.", prefill: 256, decode: 128)
        }
    }

    public func chat(instructions: String, constrained: Bool) async throws(ModelRuntimeError) -> any EngineChat {
        LlamaChat(session: session, instructions: instructions)
    }

    private static func open(_ model: InstalledModel, on backend: ModelBackend, profile: ModelProfile)
        async throws(ModelRuntimeError) -> LlamaSession
    {
        let files = model.spec.source?.files.filter { $0.path.lowercased().hasSuffix(".gguf") } ?? []
        guard let weights = files.first(where: { !ModelResolution.isEncoder($0.path) }) else {
            throw .unavailable(.notDownloaded)
        }
        let path = model.location(of: weights).path
        let projector = files.first { ModelResolution.isEncoder($0.path) }.map { model.location(of: $0).path }
        for file in [path] + (projector.map { [$0] } ?? []) where !FileManager.default.fileExists(atPath: file) {
            throw .unavailable(.notDownloaded)
        }
        return try await LlamaSession.open(
            path: path, projector: projector, backend: backend, window: model.window, profile: profile)
    }
}

/// One conversation with a llama.cpp model. Its history stays in the
/// session until it's let go of.
final class LlamaChat: EngineChat {
    /// The most a turn thinks before it answers, in tokens.
    static let thinkingTokens = 512
    /// How a turn picks its tokens when the model's profile doesn't say.
    static let sampling = ModelProfile.Sampling(topK: 40, topP: 0.95, temperature: 0.7)

    private let session: LlamaSession
    private let instructions: String
    private let id = UUID()
    private let stop = LlamaStop()

    init(session: LlamaSession, instructions: String) {
        self.session = session
        self.instructions = instructions
    }

    deinit {
        session.forget(id)
    }

    func send(
        _ message: String, attachments: [ModelAttachment], format: String?, thinks: Bool,
        thought: (@Sendable (ModelThinking) -> Void)?, partial: @escaping @Sendable (String) -> Void
    ) async throws(ModelRuntimeError) -> String {
        if attachments.contains(where: { $0.kind == .audio }) { throw .unsupportedInput(.audio) }
        let images = attachments.filter { $0.kind == .image }.map(\.file)
        let grammar: String?
        do throws(SchemaGrammarError) {
            grammar = try format.map { schema throws(SchemaGrammarError) in try SchemaGrammar.gbnf(for: schema) }
        } catch {
            throw .failed(error.description)
        }
        let start = ContinuousClock.now
        let turn = Turn(
            id: id, instructions: instructions, message: message, images: images, grammar: grammar, thinks: thinks,
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
    var images: [URL]
    var grammar: String?
    var thinks: Bool
    var stop: LlamaStop
    var thought: (@Sendable (ModelThinking) -> Void)?
    var partial: @Sendable (String) -> Void

    /// The answer, and the reasoning to show for it, if the model showed
    /// any.
    func run(on session: LlamaSession) throws(ModelRuntimeError) -> (String, String?) {
        try prompt(on: session)
        let thinking = session.profile.thinking
        let sampling = session.profile.sampling ?? LlamaChat.sampling
        // A model that always thinks does, but its reasoning is shown only
        // when the turn asked for it.
        var shown: String?
        if let thinking, thinks || thinking.off == nil {
            let reasoning = try think(on: session, thinking: thinking, sampling: sampling)
            guard let reasoning else { return ("", thinks ? "" : nil) }
            shown = thinks ? reasoning : nil
        }
        let answer = try answer(on: session, sampler: session.sampler(grammar: grammar, sampling: sampling))
        return (answer, shown ?? (thinks ? "" : nil))
    }

    /// Gives the model the message, after the instructions if it's the
    /// first, and starts its answer.
    private func prompt(on session: LlamaSession) throws(ModelRuntimeError) {
        if session.isNew(id) {
            if instructions.isEmpty {
                try session.give("<|im_start|>user\n", special: true, in: id, beginning: true)
            } else {
                try session.give("<|im_start|>system\n", special: true, in: id, beginning: true)
                try session.give(instructions, special: false, in: id)
                try session.give("<|im_end|>\n<|im_start|>user\n", special: true, in: id)
            }
        } else {
            // The model's last answer ended where it wrote its end token,
            // which isn't read back.
            try session.give("<|im_end|>\n<|im_start|>user\n", special: true, in: id)
        }
        try session.give(images: images, in: id)
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
        on session: LlamaSession, thinking: ModelProfile.Thinking, sampling: ModelProfile.Sampling
    ) throws(ModelRuntimeError) -> String? {
        let sampler = try session.sampler(grammar: nil, sampling: sampling)
        var bytes: [UInt8] = []
        for _ in 0..<LlamaChat.thinkingTokens {
            if stop.isStopped { throw .cancelled }
            guard session.room > 8 else { throw .exceededContext }
            let (token, ends) = session.next(sampler)
            if ends { return nil }
            try session.give([token], in: id)
            bytes += session.bytes(of: token, special: true)
            let text = lossy(bytes)
            if let end = text.range(of: thinking.end) {
                try session.give("\n\n", special: false, in: id)
                return trimmed(String(text[..<end.lowerBound]))
            }
            if thinks { thought?(ModelThinking(text: partialText(text), processor: session.backend)) }
        }
        // Out of budget: it's stopped thinking for it.
        try session.give("\n\(thinking.end)\n\n", special: true, in: id)
        return trimmed(lossy(bytes))
    }

    /// The model's answer, until it ends its turn.
    private func answer(on session: LlamaSession, sampler: LlamaSampler) throws(ModelRuntimeError) -> String {
        var bytes: [UInt8] = []
        while true {
            if stop.isStopped { throw .cancelled }
            guard session.room > 1 else { throw .exceededContext }
            let (token, ends) = session.next(sampler)
            if ends { break }
            try session.give([token], in: id)
            bytes += session.bytes(of: token, special: false)
            partial(partialText(lossy(bytes)))
        }
        return trimmed(lossy(bytes))
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

/// UTF-8 as text, with a replacement character for each sequence that isn't
/// whole yet, as the last character of an answer still being written can
/// be.
private func lossy(_ bytes: [UInt8]) -> String {
    // swiftlint:disable:next optional_data_string_conversion
    String(decoding: bytes, as: UTF8.self)
}
