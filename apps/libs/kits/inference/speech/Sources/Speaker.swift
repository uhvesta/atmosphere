import Foundation
import InferenceKit

/// Where speech plays: the device's speaker, or a recording in tests.
public protocol AudioOutput: Sendable {
    /// Gets ready to play audio at `sampleRate`, before the first piece.
    func start(sampleRate: Double) async throws(SpeechError)
    /// Plays `audio` after what's already queued.
    func play(_ audio: SpeechAudio) async
    /// Returns once everything queued has played, or it's stopped.
    func drain() async
    /// Stops at once and drops what's queued.
    func stop() async
}

/// Reads text aloud in the voice the person picked for its language, one
/// text at a time: speaking another, or `stop`, cuts off the one playing.
/// A picked voice model that isn't downloaded, or doesn't speak the
/// language, gives way to the system voice; see `SpeechRoute`.
public final class Speaker: Sendable {
    private let installed: @Sendable () async -> [InstalledModel]
    private let selection: VoiceSelection
    private let system: any SpeechEngine
    private let output: any AudioOutput
    private let makeEngine: @Sendable (InstalledModel, VoiceModel) -> any SpeechEngine
    private let catalog: [VoiceModel]
    /// The engine for each voice model loaded, so its parts load once.
    private let engines = Locked<[ModelID: (Date, any SpeechEngine)]>([:])
    /// What's speaking now.
    private let current = Locked<Task<Result<Void, SpeechError>, Never>?>(nil)

    /// - Parameters:
    ///   - installed: The models downloaded now.
    ///   - selection: The voice picked for each language.
    ///   - system: The system voice, which needs no download.
    ///   - output: Where the audio plays.
    ///   - catalog: The voice models there are.
    ///   - engine: The engine for a downloaded voice model.
    public init(
        installed: @escaping @Sendable () async -> [InstalledModel], selection: VoiceSelection,
        system: any SpeechEngine, output: any AudioOutput, catalog: [VoiceModel] = VoiceCatalog.all,
        engine: @escaping @Sendable (InstalledModel, VoiceModel) -> any SpeechEngine
    ) {
        self.installed = installed
        self.selection = selection
        self.system = system
        self.output = output
        self.catalog = catalog
        makeEngine = engine
    }

    /// What would speak `language` now.
    public func route(for language: String) async -> SpeechRoute {
        SpeechRoute.resolve(
            selection.choice(for: language), language: language, installed: await installed(), catalog: catalog)
    }

    /// Reads `text` aloud in `language`, a BCP 47 tag, after stopping
    /// whatever is speaking, and returns once it has played, or once it's
    /// cut off by `stop`, another text or the task's cancellation.
    public func speak(_ text: String, language: String) async throws(SpeechError) {
        let task = current.withValue { current in
            let previous = current
            previous?.cancel()
            let task = Task<Result<Void, SpeechError>, Never> {
                // The one before it stops first, so the two never overlap.
                _ = await previous?.value
                return await self.run(text, language: language)
            }
            current = task
            return task
        }
        let result = await withTaskCancellationHandler {
            await task.value
        } onCancel: {
            task.cancel()
        }
        current.withValue { current in
            if current == task { current = nil }
        }
        try result.get()
    }

    /// Stops what's speaking, at once.
    public func stop() async {
        let previous = current.withValue { current in
            let previous = current
            current = nil
            return previous
        }
        previous?.cancel()
        _ = await previous?.value
        await output.stop()
    }

    private func run(_ text: String, language: String) async -> Result<Void, SpeechError> {
        guard !Task.isCancelled else { return .success(()) }
        let engine: any SpeechEngine
        let voice: String
        switch await route(for: language) {
        case .system:
            engine = system
            voice = ""
        case .model(let model, let entry, let picked):
            engine = self.engine(for: model, entry)
            voice = picked
        }
        let (pieces, sink) = AsyncStream.makeStream(of: SpeechAudio.self)
        let making = Task<SpeechError?, Never> {
            defer { sink.finish() }
            do throws(SpeechError) {
                try await engine.synthesize(text, language: language, voice: voice) { sink.yield($0) }
                return nil
            } catch {
                return error
            }
        }
        return await withTaskCancellationHandler {
            var started = false
            for await piece in pieces {
                guard !Task.isCancelled else { break }
                if !started {
                    do throws(SpeechError) {
                        try await output.start(sampleRate: piece.sampleRate)
                    } catch {
                        making.cancel()
                        return .failure(error)
                    }
                    started = true
                }
                await output.play(piece)
            }
            if let error = await making.value {
                await output.stop()
                return .failure(error)
            }
            if Task.isCancelled {
                await output.stop()
            } else if started {
                await output.drain()
            }
            if Task.isCancelled { await output.stop() }
            return .success(())
        } onCancel: {
            making.cancel()
        }
    }

    /// The engine for `model`, loaded once for each download of it.
    private func engine(for model: InstalledModel, _ entry: VoiceModel) -> any SpeechEngine {
        engines.withValue { engines in
            if let (installedAt, engine) = engines[model.id], installedAt == model.installedAt { return engine }
            let engine = makeEngine(model, entry)
            engines[model.id] = (model.installedAt, engine)
            return engine
        }
    }
}
