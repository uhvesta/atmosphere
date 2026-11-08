import Foundation
import InferenceKit
import MLX
import MLXGuidedGeneration
import MLXLLM
import MLXLMCommon
import Synchronization

/// One model loaded by MLX, with the cache its conversations take turns
/// in. MLX's model and cache aren't safe to change from several threads
/// at once, so every call goes through one serial queue, and everything
/// that changes is changed only there.
final class MLXSession: @unchecked Sendable {
    typealias Token = Int

    let backend: ModelBackend
    let profile: ModelProfile
    private let context: ModelContext
    private let queue: DispatchQueue
    private let window: Int
    /// The tokens that end the model's turn.
    private let stops: Set<Token>
    /// The token that ends a turn, as the grammar's matcher knows it.
    private let end: Token
    /// The vocabulary as the grammar's matcher reads it, made the first
    /// time an answer has a schema, since it takes a pass over every token.
    private var grammarTokenizer: GrammarTokenizer?

    /// Whose history the cache holds, and how far into it.
    private var loaded: UUID?
    private var cache: [any KVCache] = []
    private var state: LMOutput.State?
    /// The likelihoods of the token after the last one given.
    private var logits: MLXArray?
    private var position = 0
    /// What each conversation has given the model so far, to give it again
    /// when another conversation used the cache in between.
    private var histories: [UUID: [Token]] = [:]
    private var steps = 0
    /// What stops the call running now.
    private let stop = Mutex<MLXStop?>(nil)

    private init(
        context: ModelContext, queue: DispatchQueue, window: Int, backend: ModelBackend, profile: ModelProfile
    ) throws(ModelRuntimeError) {
        self.context = context
        self.queue = queue
        self.window = window
        self.backend = backend
        self.profile = profile
        let tokenizer = context.tokenizer
        var stops = context.configuration.eosTokenIds
        for token in ["<|im_end|>", "<|endoftext|>"] {
            if let id = tokenizer.convertTokenToId(token) { stops.insert(id) }
        }
        if let id = tokenizer.eosTokenId { stops.insert(id) }
        guard let end = tokenizer.convertTokenToId("<|im_end|>") ?? tokenizer.eosTokenId else {
            throw .failed("the model's tokenizer has no token that ends a turn")
        }
        self.stops = stops
        self.end = end
    }

    // MARK: Loading

    /// Loads the model in `directory` to run on `backend` with a window of
    /// `window` tokens.
    static func open(directory: URL, backend: ModelBackend, window: Int, profile: ModelProfile)
        async throws(ModelRuntimeError) -> MLXSession
    {
        let loaded: ModelContext
        do {
            loaded = try await LLMModelFactory.shared.load(from: directory, using: TransformersLoader())
        } catch {
            throw .failed("MLX couldn't load the model in \(directory.lastPathComponent) (\(error))")
        }
        let queue = DispatchQueue(label: "InferenceKitMLX.session", qos: .userInitiated)
        return try MLXSession(context: loaded, queue: queue, window: window, backend: backend, profile: profile)
    }

    // MARK: Running

    /// Runs `work` on the session's queue, on the GPU, which `stop` stops.
    func run<Value: Sendable>(
        stoppedBy stop: MLXStop,
        _ work: @escaping @Sendable (MLXSession) throws(ModelRuntimeError) -> Value
    ) async throws(ModelRuntimeError) -> Value {
        let result: Result<Value, ModelRuntimeError> = await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                queue.async {
                    self.stop.withLock { $0 = stop }
                    let result = Device.withDefaultDevice(.gpu) {
                        Result { () throws(ModelRuntimeError) -> Value in
                            if stop.isStopped { throw .cancelled }
                            return try work(self)
                        }
                    }
                    self.stop.withLock { $0 = nil }
                    continuation.resume(returning: result)
                }
            }
        } onCancel: {
            stop.stop()
        }
        return try result.get()
    }

    /// Lets go of a conversation's history.
    func forget(_ chat: UUID) {
        queue.async {
            self.histories[chat] = nil
            if self.loaded == chat { self.clear() }
        }
    }

    /// Whether `chat` hasn't given the model anything yet.
    func isNew(_ chat: UUID) -> Bool {
        histories[chat]?.isEmpty ?? true
    }

    /// Puts `chat`'s history in the cache, if another conversation's is
    /// there, so what's given next follows on from it.
    func switchTo(_ chat: UUID) throws(ModelRuntimeError) {
        guard loaded != chat else { return }
        clear()
        if let history = histories[chat], !history.isEmpty { try decode(history) }
        loaded = chat
    }

    /// Gives the model `text` in `chat`, after what it has. `special` when
    /// the text is the prompt's own markup, such as <|im_start|>, which
    /// what the person wrote mustn't be read as.
    func give(_ text: String, special: Bool, in chat: UUID) throws(ModelRuntimeError) {
        try give(tokenize(text, special: special), in: chat)
    }

    func give(_ tokens: [Token], in chat: UUID) throws(ModelRuntimeError) {
        guard !tokens.isEmpty else { return }
        try switchTo(chat)
        do throws(ModelRuntimeError) {
            try decode(tokens)
        } catch {
            clear()
            throw error
        }
        histories[chat, default: []] += tokens
    }

    /// A sampler for the next tokens, held to the JSON Schema `schema`
    /// when there is one.
    func sampler(schema: String?, sampling: ModelProfile.Sampling) throws(ModelRuntimeError) -> MLXSampler {
        let picker: any LogitSampler =
            if sampling.temperature <= 0 || sampling.topK == 1 {
                ArgMaxSampler()
            } else {
                TopPSampler(temperature: sampling.temperature, topP: sampling.topP, topK: sampling.topK)
            }
        guard let schema else { return MLXSampler(picker: picker, grammar: nil) }
        do {
            let tokenizer = try grammarTokenizer ?? makeGrammarTokenizer()
            grammarTokenizer = tokenizer
            return MLXSampler(
                picker: picker, grammar: try GrammarConstraint(tokenizer: tokenizer, jsonSchema: schema))
        } catch {
            throw .failed("MLX couldn't read the answer's JSON Schema (\(error))")
        }
    }

    /// The next token `sampler` picks after what the cache holds, and
    /// whether it ends the model's turn.
    func next(_ sampler: MLXSampler) throws(ModelRuntimeError) -> (token: Token, ends: Bool) {
        guard var logits else { throw .failed("MLX was asked for a token before it read anything") }
        if let grammar = sampler.grammar {
            let mask: MaskResult
            do {
                mask = try grammar.computeMask()
            } catch {
                throw .failed("MLX couldn't keep the answer to its schema (\(error))")
            }
            if mask.isTerminated { return (end, true) }
            if mask.needsApply { logits += Self.additive(mask, logits: logits.dim(-1)) }
        }
        let token = sampler.picker.sample(logits: logits).item(Int.self)
        let ends = stops.contains(token)
        if let grammar = sampler.grammar, !ends {
            do {
                _ = try grammar.commitToken(Int32(token))
            } catch {
                throw .decodingFailure
            }
        }
        return (token, ends)
    }

    /// What `tokens` read as. `special` writes markup tokens out, such as
    /// a <think> that a model's vocabulary has as one.
    func text(of tokens: [Token], special: Bool) -> String {
        context.tokenizer.decode(tokenIds: tokens, skipSpecialTokens: !special)
    }

    /// How many more tokens fit in the window.
    var room: Int { window - position }

    // MARK: Measuring

    /// Reads `prompt` cut or repeated to `prefill` tokens, then writes
    /// `decode` tokens, taking the likeliest each time, from an empty
    /// cache, and times both.
    func benchmark(prompt: String, prefill: Int, decode decodeCount: Int) throws(ModelRuntimeError) -> EngineSpeed {
        guard prefill + decodeCount < window else { throw .exceededContext }
        let sentence = tokenize(prompt + " ", special: false)
        guard !sentence.isEmpty else { throw .failed("the model read the measuring prompt as nothing") }
        var tokens: [Token] = []
        while tokens.count < prefill { tokens += sentence }
        tokens = Array(tokens.prefix(prefill))
        clear()
        defer { clear() }
        let greedy = MLXSampler(picker: ArgMaxSampler(), grammar: nil)
        let clock = ContinuousClock()
        let start = clock.now
        try decode(tokens)
        var token = try next(greedy).token
        let first = clock.now
        for _ in 0..<decodeCount {
            try decode([token])
            token = try next(greedy).token
        }
        let end = clock.now
        let reading = (first - start).seconds
        let writing = (end - first).seconds
        return EngineSpeed(
            prefillTokens: prefill, decodeTokens: decodeCount,
            prefillTokensPerSecond: reading > 0 ? Double(prefill) / reading : 0,
            decodeTokensPerSecond: writing > 0 ? Double(decodeCount) / writing : 0, timeToFirstToken: reading)
    }

    // MARK: MLX

    /// `text` as tokens. Text that isn't markup has the start of any
    /// markup in it broken, since the tokenizer reads markup wherever it
    /// is: a message with <|im_end|> in it would otherwise end its turn.
    private func tokenize(_ text: String, special: Bool) -> [Token] {
        let text = special ? text : text.replacingOccurrences(of: "<|", with: "<\u{200B}|")
        return context.tokenizer.encode(text: text, addSpecialTokens: false)
    }

    /// Reads `tokens` into the cache after what it holds, with the
    /// likelihoods of what follows the last.
    private func decode(_ tokens: [Token]) throws(ModelRuntimeError) {
        guard position + tokens.count < window else { throw .exceededContext }
        let model = context.model
        do {
            if cache.isEmpty { cache = try model.newCache(parameters: nil) }
            try autoreleasepool { () throws in
                let input = LMInput(tokens: MLXArray(tokens.map(Int32.init)))
                let output: LMOutput
                switch try model.prepare(input, cache: cache, state: state, prefill: PrefillParameters()) {
                case .tokens(let rest): output = model(rest[text: .newAxis], cache: cache, state: state)
                case .logits(let result): output = result
                }
                state = output.state
                let last = output.logits[0..., -1, 0...]
                // The cache changes as a chain of operations, which keeps
                // every step's arrays until it's worked out.
                eval([last] + cache.flatMap(\.state))
                logits = last
            }
        } catch {
            throw .failed("MLX couldn't read the prompt (\(error))")
        }
        position += tokens.count
        steps += 1
        // MLX keeps freed buffers to reuse; ones of sizes that only grow
        // never are.
        if steps.isMultiple(of: 256) { Memory.clearCache() }
    }

    /// Empties the cache, for another conversation or none.
    private func clear() {
        cache = []
        state = nil
        logits = nil
        position = 0
        loaded = nil
    }

    private func makeGrammarTokenizer() throws -> GrammarTokenizer {
        let vocab = TokenizerVocabExtractor.extractForGrammar(from: context.tokenizer)
        return try GrammarTokenizer(vocab: vocab.vocab, vocabType: vocab.vocabType, eosTokenId: Int32(end))
    }

    /// The grammar's bitmask as an addition to the logits: nothing for
    /// each token it allows, and minus infinity for each it doesn't.
    private static func additive(_ mask: MaskResult, logits count: Int) -> MLXArray {
        var values = [Float](repeating: -.infinity, count: count)
        let bits = min(mask.mask.count * 32, count)
        for index in 0..<bits where (UInt32(bitPattern: mask.mask[index / 32]) >> UInt32(index % 32)) & 1 == 1 {
            values[index] = 0
        }
        return MLXArray(values)
    }
}

/// How a session picks tokens, held to a grammar or not.
struct MLXSampler {
    var picker: any LogitSampler
    var grammar: GrammarConstraint?
}

/// Whether the call a session is running was stopped.
final class MLXStop: Sendable {
    private let stopped = Atomic(false)

    var isStopped: Bool { stopped.load(ordering: .relaxed) }

    func stop() { stopped.store(true, ordering: .relaxed) }
}

extension Duration {
    fileprivate var seconds: Double {
        let (seconds, attoseconds) = components
        return Double(seconds) + Double(attoseconds) / 1e18
    }
}
