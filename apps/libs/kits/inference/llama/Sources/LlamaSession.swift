import Darwin
import Foundation
import InferenceKit
import Synchronization
import llama

/// One model loaded by llama.cpp, with the context its conversations take
/// turns in. llama.cpp isn't safe to call from several threads at once, so
/// every call goes through one serial queue, and everything that changes
/// is changed only there.
final class LlamaSession: @unchecked Sendable {
    typealias Token = llama_token

    /// What a conversation has given the model so far, to give it again
    /// when another conversation used the context in between.
    struct History {
        enum Piece {
            case tokens([Token])
            case images([URL])
        }

        var pieces: [Piece] = []
        var isEmpty: Bool { pieces.isEmpty }

        /// Adds `tokens`, to the tokens before them if the last piece is
        /// tokens, as each token of an answer is.
        mutating func append(_ tokens: [Token]) {
            if case .tokens(var earlier)? = pieces.last {
                // Taken out first, so it grows in place.
                pieces.removeLast()
                earlier += tokens
                pieces.append(.tokens(earlier))
            } else {
                pieces.append(.tokens(tokens))
            }
        }
    }

    let backend: ModelBackend
    let profile: ModelProfile
    private let model: OpaquePointer
    private let context: OpaquePointer
    private let vocab: OpaquePointer
    /// The vision encoder, for a model that reads photos.
    private let vision: OpaquePointer?
    private let queue: DispatchQueue
    private var batch: llama_batch
    private let window: Int32
    private let batchSize: Int32
    /// The token that starts a prompt, for a model whose template writes
    /// one.
    private let begin: Token?

    /// Whose history the context holds, and how far into it.
    private var loaded: UUID?
    private var position: llama_pos = 0
    private var histories: [UUID: History] = [:]
    /// What stops the call running now.
    private let stop = Mutex<LlamaStop?>(nil)

    private init(
        model: OpaquePointer, context: OpaquePointer, vocab: OpaquePointer, vision: OpaquePointer?,
        queue: DispatchQueue, batch: llama_batch, batchSize: Int32, window: Int32, backend: ModelBackend,
        profile: ModelProfile
    ) {
        self.model = model
        self.context = context
        self.vocab = vocab
        self.vision = vision
        self.queue = queue
        self.batch = batch
        self.window = window
        self.backend = backend
        self.profile = profile
        self.batchSize = batchSize
        let template = llama_model_chat_template(model, nil).map { String(cString: $0) } ?? ""
        let bos = llama_vocab_bos(vocab)
        begin =
            bos != LLAMA_TOKEN_NULL && (llama_vocab_get_add_bos(vocab) || template.contains("bos_token"))
            ? bos : nil
    }

    deinit {
        llama_batch_free(batch)
        if let vision { mtmd_free(vision) }
        llama_free(context)
        llama_model_free(model)
    }

    // MARK: Loading

    /// Loads the GGUF file at `path` to run on `backend` with a window of
    /// `window` tokens, and its vision encoder from `projector` if it has
    /// one, with `options` where they're set and the usual settings
    /// where they aren't.
    static func open(
        path: String, projector: String?, backend: ModelBackend, window: Int, profile: ModelProfile,
        options: EngineOptions?
    ) async throws(ModelRuntimeError) -> LlamaSession {
        let queue = DispatchQueue(label: "InferenceKitLlama.session", qos: .userInitiated)
        let result: Result<LlamaSession, ModelRuntimeError> = await withCheckedContinuation { continuation in
            queue.async {
                continuation.resume(
                    returning: Result { () throws(ModelRuntimeError) -> LlamaSession in
                        try load(
                            path: path, projector: projector, backend: backend, window: window, profile: profile,
                            options: options, queue: queue)
                    })
            }
        }
        return try result.get()
    }

    private static func load(
        path: String, projector: String?, backend: ModelBackend, window: Int, profile: ModelProfile,
        options: EngineOptions?, queue: DispatchQueue
    ) throws(ModelRuntimeError) -> LlamaSession {
        LlamaBackend.start()
        let threads = Int32(profile.cpuThreads ?? LlamaBackend.performanceCores)
        var modelParams = llama_model_default_params()
        // Every layer on the GPU unless the person set fewer.
        modelParams.n_gpu_layers = backend == .gpu ? Int32(clamping: options?.gpuLayers ?? -1) : 0
        // On the CPU, llama.cpp is told of no other device, so nothing of
        // the model goes to the GPU.
        var devices: [ggml_backend_dev_t?] = [ggml_backend_dev_by_type(GGML_BACKEND_DEVICE_TYPE_CPU), nil]
        let loadedModel = devices.withUnsafeMutableBufferPointer { list in
            if backend == .cpu { modelParams.devices = list.baseAddress }
            return llama_model_load_from_file(path, modelParams)
        }
        guard let model = loadedModel else {
            throw .failed("llama.cpp couldn't load \((path as NSString).lastPathComponent)")
        }
        var contextParams = llama_context_default_params()
        contextParams.n_ctx = UInt32(window)
        // The prompt tokens a batch reads at once.
        let batchTokens = Int32(clamping: options?.batchTokens ?? EngineOptions.llamaBatchTokens)
        contextParams.n_batch = UInt32(batchTokens)
        contextParams.n_ubatch = UInt32(clamping: options?.microBatchTokens ?? Int(batchTokens))
        contextParams.n_seq_max = 1
        contextParams.n_threads = threads
        contextParams.n_threads_batch = threads
        // The window's keys and values at 8 bits unless set, which takes
        // half the memory of 16 for answers that read the same; llama.cpp
        // needs flash attention for a cache under 16 bits.
        let flash = options?.flashAttention ?? true
        contextParams.flash_attn_type = flash ? LLAMA_FLASH_ATTN_TYPE_ENABLED : LLAMA_FLASH_ATTN_TYPE_DISABLED
        let cache: ggml_type =
            switch options?.kvCache ?? .q8_0 {
            case .f16: GGML_TYPE_F16
            case .q8_0: GGML_TYPE_Q8_0
            case .q4_0: GGML_TYPE_Q4_0
            }
        contextParams.type_k = cache
        contextParams.type_v = cache
        contextParams.offload_kqv = backend == .gpu
        contextParams.op_offload = backend == .gpu
        contextParams.no_perf = false
        guard let context = llama_init_from_model(model, contextParams) else {
            llama_model_free(model)
            throw .failed("llama.cpp couldn't make room for a window of \(window) tokens")
        }
        guard let vocab = llama_model_get_vocab(model) else {
            llama_free(context)
            llama_model_free(model)
            throw .failed("the model has no vocabulary")
        }
        var vision: OpaquePointer?
        if let projector {
            var visionParams = mtmd_context_params_default()
            visionParams.use_gpu = backend == .gpu && profile.vision == .withModel
            visionParams.n_threads = threads
            visionParams.flash_attn_type = LLAMA_FLASH_ATTN_TYPE_ENABLED
            visionParams.warmup = false
            vision = mtmd_init_from_file(projector, model, visionParams)
            guard vision != nil else {
                llama_free(context)
                llama_model_free(model)
                throw .failed("llama.cpp couldn't load the model's vision encoder")
            }
        }
        let session = LlamaSession(
            model: model, context: context, vocab: vocab, vision: vision, queue: queue,
            batch: llama_batch_init(batchTokens, 0, 1), batchSize: batchTokens, window: Int32(window), backend: backend,
            profile: profile)
        llama_set_abort_callback(context, LlamaSession.shouldAbort, Unmanaged.passUnretained(session).toOpaque())
        return session
    }

    /// Whether llama.cpp should stop the computation it's in, because the
    /// call it's for was stopped.
    private static let shouldAbort: ggml_abort_callback = { data in
        guard let data else { return false }
        let session = Unmanaged<LlamaSession>.fromOpaque(data).takeUnretainedValue()
        return session.stop.withLock { $0?.isStopped ?? false }
    }

    // MARK: Running

    /// Runs `work` on the session's queue, which `stop` stops.
    func run<Value: Sendable>(
        stoppedBy stop: LlamaStop,
        _ work: @escaping @Sendable (LlamaSession) throws(ModelRuntimeError) -> Value
    ) async throws(ModelRuntimeError) -> Value {
        let result: Result<Value, ModelRuntimeError> = await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                queue.async {
                    self.stop.withLock { $0 = stop }
                    let result = Result { () throws(ModelRuntimeError) -> Value in
                        if stop.isStopped { throw .cancelled }
                        return try work(self)
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
            if self.loaded == chat { self.loaded = nil }
        }
    }

    /// Whether `chat` hasn't given the model anything yet.
    func isNew(_ chat: UUID) -> Bool {
        histories[chat]?.isEmpty ?? true
    }

    /// Puts `chat`'s history in the context, if another conversation's is
    /// there, so what's given next follows on from it.
    func switchTo(_ chat: UUID) throws(ModelRuntimeError) {
        guard loaded != chat else { return }
        llama_memory_clear(llama_get_memory(context), true)
        loaded = nil
        position = 0
        let pieces = histories[chat]?.pieces ?? []
        for piece in pieces {
            switch piece {
            case .tokens(let tokens): try decode(tokens)
            case .images(let images): try decode(images: images)
            }
        }
        loaded = chat
    }

    /// Gives the model `text` in `chat`, after what it has. `special` when
    /// the text is the prompt's own markup, such as <|im_start|>, which
    /// what the person wrote mustn't be read as.
    func give(_ text: String, special: Bool, in chat: UUID, beginning: Bool = false) throws(ModelRuntimeError) {
        var tokens = tokenize(text, special: special)
        if beginning, let begin { tokens.insert(begin, at: 0) }
        try give(tokens, in: chat)
    }

    func give(_ tokens: [Token], in chat: UUID) throws(ModelRuntimeError) {
        guard !tokens.isEmpty else { return }
        try switchTo(chat)
        do throws(ModelRuntimeError) {
            try decode(tokens)
        } catch {
            loaded = nil
            throw error
        }
        histories[chat, default: History()].append(tokens)
    }

    /// Gives the model photos in `chat`, through its vision encoder.
    func give(images: [URL], in chat: UUID) throws(ModelRuntimeError) {
        guard !images.isEmpty else { return }
        guard vision != nil else { throw .unsupportedInput(.image) }
        try switchTo(chat)
        do throws(ModelRuntimeError) {
            try decode(images: images)
        } catch {
            loaded = nil
            throw error
        }
        histories[chat, default: History()].pieces.append(.images(images))
    }

    /// A sampler for the next tokens, held to `grammar` when there is one.
    func sampler(grammar: String?, sampling: ModelProfile.Sampling) throws(ModelRuntimeError) -> LlamaSampler {
        guard let chain = llama_sampler_chain_init(llama_sampler_chain_default_params()) else {
            throw .failed("llama.cpp couldn't make a sampler")
        }
        let sampler = LlamaSampler(chain)
        if let grammar {
            guard let constraint = llama_sampler_init_grammar(vocab, grammar, "root") else {
                throw .failed("llama.cpp couldn't read the grammar for the answer's JSON Schema")
            }
            llama_sampler_chain_add(chain, constraint)
        }
        if sampling.temperature <= 0 || sampling.topK == 1 {
            llama_sampler_chain_add(chain, llama_sampler_init_greedy())
        } else {
            llama_sampler_chain_add(chain, llama_sampler_init_top_k(Int32(sampling.topK)))
            llama_sampler_chain_add(chain, llama_sampler_init_top_p(sampling.topP, 1))
            llama_sampler_chain_add(chain, llama_sampler_init_temp(sampling.temperature))
            llama_sampler_chain_add(chain, llama_sampler_init_dist(UInt32.random(in: 0...UInt32.max)))
        }
        return sampler
    }

    /// The next token `sampler` picks after what the context holds, and
    /// whether it ends the model's turn.
    func next(_ sampler: LlamaSampler) -> (token: Token, ends: Bool) {
        let token = llama_sampler_sample(sampler.chain, context, -1)
        return (token, llama_vocab_is_eog(vocab, token))
    }

    /// What `token` reads as. `special` writes markup tokens out, such as
    /// a <think> that a model's vocabulary has as one.
    func bytes(of token: Token, special: Bool) -> [UInt8] {
        var buffer = [CChar](repeating: 0, count: 64)
        var count = llama_token_to_piece(vocab, token, &buffer, Int32(buffer.count), 0, special)
        if count < 0 {
            buffer = [CChar](repeating: 0, count: Int(-count))
            count = llama_token_to_piece(vocab, token, &buffer, Int32(buffer.count), 0, special)
        }
        return buffer.prefix(Int(max(count, 0))).map { UInt8(bitPattern: $0) }
    }

    /// How many more tokens fit in the window.
    var room: Int { Int(window - position) }

    // MARK: Measuring

    /// Reads `prompt` cut or repeated to `prefill` tokens, then writes
    /// `decode` tokens, taking the likeliest each time, from an empty
    /// context, and times both.
    func benchmark(prompt: String, prefill: Int, decode decodeCount: Int) throws(ModelRuntimeError) -> EngineSpeed {
        guard prefill + decodeCount < Int(window) else { throw .exceededContext }
        var tokens: [Token] = []
        let sentence = tokenize(prompt + " ", special: false)
        guard !sentence.isEmpty else { throw .failed("the model read the measuring prompt as nothing") }
        while tokens.count < prefill { tokens += sentence }
        tokens = Array(tokens.prefix(prefill))
        llama_memory_clear(llama_get_memory(context), true)
        loaded = nil
        position = 0
        let sampler = try sampler(grammar: nil, sampling: .greedy)
        let clock = ContinuousClock()
        let start = clock.now
        try decode(tokens)
        var token = next(sampler).token
        let first = clock.now
        for _ in 0..<decodeCount {
            try decode([token])
            token = next(sampler).token
        }
        let end = clock.now
        llama_memory_clear(llama_get_memory(context), true)
        position = 0
        let reading = (first - start).seconds
        let writing = (end - first).seconds
        return EngineSpeed(
            prefillTokens: prefill, decodeTokens: decodeCount,
            prefillTokensPerSecond: reading > 0 ? Double(prefill) / reading : 0,
            decodeTokensPerSecond: writing > 0 ? Double(decodeCount) / writing : 0, timeToFirstToken: reading)
    }

    // MARK: llama.cpp

    func tokenize(_ text: String, special: Bool) -> [Token] {
        let length = Int32(text.utf8.count)
        var tokens = [Token](repeating: 0, count: Int(length) + 2)
        var count = llama_tokenize(vocab, text, length, &tokens, Int32(tokens.count), false, special)
        if count < 0 {
            tokens = [Token](repeating: 0, count: Int(-count))
            count = llama_tokenize(vocab, text, length, &tokens, Int32(tokens.count), false, special)
        }
        return Array(tokens.prefix(Int(max(count, 0))))
    }

    /// Reads `tokens` into the context after what it holds, a batch at a
    /// time, with the likelihoods of what follows the last.
    private func decode(_ tokens: [Token]) throws(ModelRuntimeError) {
        guard Int(position) + tokens.count < Int(window) else { throw .exceededContext }
        guard let ids = batch.token, let positions = batch.pos, let counts = batch.n_seq_id,
            let sequences = batch.seq_id, let outputs = batch.logits
        else { throw .failed("llama.cpp's batch has no room") }
        var start = 0
        while start < tokens.count {
            let count = min(Int(batchSize), tokens.count - start)
            for index in 0..<count {
                ids[index] = tokens[start + index]
                positions[index] = position + llama_pos(index)
                counts[index] = 1
                sequences[index]?[0] = 0
                outputs[index] = start + index == tokens.count - 1 ? 1 : 0
            }
            batch.n_tokens = Int32(count)
            try check(llama_decode(context, batch))
            position += llama_pos(count)
            start += count
        }
    }

    /// Reads photos into the context through the vision encoder.
    private func decode(images: [URL]) throws(ModelRuntimeError) {
        guard let vision else { throw .unsupportedInput(.image) }
        var bitmaps: [OpaquePointer?] = []
        defer { for bitmap in bitmaps { mtmd_bitmap_free(bitmap) } }
        for image in images {
            let read = mtmd_helper_bitmap_init_from_file(vision, image.path, false, mtmd_helper_init_opt_default())
            if let video = read.video_ctx { mtmd_helper_video_free(video) }
            guard let bitmap = read.bitmap else {
                throw .failed("llama.cpp couldn't read the photo \(image.lastPathComponent)")
            }
            bitmaps.append(bitmap)
        }
        guard let chunks = mtmd_input_chunks_init() else { throw .failed("llama.cpp couldn't read the photos") }
        defer { mtmd_input_chunks_free(chunks) }
        let marker = String(cString: mtmd_default_marker())
        let text = Array(repeating: marker, count: images.count).joined(separator: "\n") + "\n"
        let tokenized = text.withCString { characters in
            var input = mtmd_input_text(
                text: characters, text_len: text.utf8.count, add_special: false, parse_special: false)
            return bitmaps.withUnsafeBufferPointer { list in
                mtmd_tokenize(vision, chunks, &input, list.baseAddress, list.count)
            }
        }
        guard tokenized == 0 else { throw .failed("llama.cpp couldn't read the photos (\(tokenized))") }
        guard position + mtmd_helper_get_n_pos(chunks) < window else { throw .exceededContext }
        var after: llama_pos = position
        try check(mtmd_helper_eval_chunks(vision, context, chunks, position, 0, batchSize, true, &after))
        position = after
    }

    private func check(_ result: Int32) throws(ModelRuntimeError) {
        switch result {
        case 0: return
        case 1: throw .exceededContext
        case 2 where stop.withLock({ $0?.isStopped ?? false }): throw .cancelled
        default: throw .failed("llama.cpp stopped reading (\(result))")
        }
    }
}

/// Whether the call a session is running was stopped.
final class LlamaStop: Sendable {
    private let stopped = Atomic(false)

    var isStopped: Bool { stopped.load(ordering: .relaxed) }

    func stop() { stopped.store(true, ordering: .relaxed) }
}

/// A llama.cpp sampler chain, freed with it.
final class LlamaSampler {
    let chain: UnsafeMutablePointer<llama_sampler>

    init(_ chain: UnsafeMutablePointer<llama_sampler>) { self.chain = chain }

    deinit { llama_sampler_free(chain) }
}

/// llama.cpp's own setup, once for the app.
enum LlamaBackend {
    private static let started: Void = {
        llama_backend_init()
        // llama.cpp logs every load and every batch to stderr.
        llama_log_set({ _, _, _ in }, nil)
        mtmd_helper_log_set({ _, _, _ in }, nil)
    }()

    static func start() { _ = started }

    /// The device's performance cores, which llama.cpp runs fastest on.
    static let performanceCores: Int = {
        var count: Int32 = 0
        var size = MemoryLayout<Int32>.size
        guard sysctlbyname("hw.perflevel0.physicalcpu", &count, &size, nil, 0) == 0, count > 0 else {
            return max(1, ProcessInfo.processInfo.activeProcessorCount / 2)
        }
        return Int(count)
    }()
}

extension Duration {
    var seconds: Double {
        let parts = components
        return Double(parts.seconds) + Double(parts.attoseconds) / 1e18
    }
}
