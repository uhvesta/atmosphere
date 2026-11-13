import Foundation
import InferenceKit
import XCTest

@testable import InferenceKitSpeech

final class SpeakerTests: XCTestCase {
    private let voiceModel = VoiceCatalog.supertonic3

    private var downloaded: InstalledModel {
        InstalledModel(
            spec: voiceModel.spec, directory: URL(filePath: "/nowhere"), installedAt: Date(timeIntervalSince1970: 0))
    }

    private func speaker(
        installed: [InstalledModel], choice: VoiceChoice, system: FakeEngine, model: FakeEngine = FakeEngine(),
        output: FakeOutput = FakeOutput()
    ) -> Speaker {
        let selection = VoiceSelection(file: nil)
        selection.choose(choice)
        return Speaker(
            installed: { installed }, selection: selection, system: system, output: output,
            engine: { _, _ in model })
    }

    func testThePickedModelSpeaksWhenItsDownloaded() async throws {
        let system = FakeEngine()
        let model = FakeEngine()
        let speaker = speaker(
            installed: [downloaded], choice: .model(voiceModel.id, voice: "M2"), system: system, model: model)

        try await speaker.speak("Hello", language: "en-US")

        XCTAssertEqual(model.spoken, [.init(text: "Hello", language: "en-US", voice: "M2")])
        XCTAssertEqual(system.spoken, [])
    }

    func testTheSystemVoiceSpeaksWhenThePickedModelIsntDownloaded() async throws {
        let system = FakeEngine()
        let model = FakeEngine()
        let speaker = speaker(installed: [], choice: .model(voiceModel.id, voice: "F1"), system: system, model: model)

        try await speaker.speak("Hallo", language: "de")

        XCTAssertEqual(system.spoken.map(\.text), ["Hallo"])
        XCTAssertEqual(model.spoken, [])
    }

    func testTheSystemVoiceSpeaksALanguageTheModelDoesnt() async throws {
        let system = FakeEngine()
        let model = FakeEngine()
        let speaker = speaker(
            installed: [downloaded], choice: .model(voiceModel.id, voice: "F1"), system: system, model: model)

        try await speaker.speak("你好", language: "zh-Hans")
        try await speaker.speak("Olá", language: "pt-BR")

        XCTAssertEqual(system.spoken.map(\.text), ["你好"])
        XCTAssertEqual(model.spoken.map(\.text), ["Olá"])
    }

    func testTheAudioMadePlaysInOrder() async throws {
        let output = FakeOutput()
        let system = FakeEngine(pieces: [[0.1, 0.2], [0.3]])
        let speaker = speaker(installed: [], choice: .system, system: system, output: output)

        try await speaker.speak("Hi", language: "en")

        XCTAssertEqual(output.events, [.start(16_000, 1), .play([0.1, 0.2]), .play([0.3]), .drain])
    }

    func testASlowReadPlaysTheSameAudioSlower() async throws {
        let output = FakeOutput()
        let system = FakeEngine(pieces: [[0.1, 0.2]])
        let speaker = speaker(installed: [], choice: .system, system: system, output: output)

        try await speaker.speak("Hi", language: "en", speed: 0.7)

        XCTAssertEqual(output.events, [.start(16_000, 0.7), .play([0.1, 0.2]), .drain])
    }

    func testAnEnginesErrorIsThrownAndStopsTheAudio() async {
        let output = FakeOutput()
        let failure = SpeechError.synthesisFailed(model: "Supertonic 3", reason: "no")
        let model = FakeEngine(pieces: [[0.5]], error: failure)
        let speaker = speaker(
            installed: [downloaded], choice: .model(voiceModel.id, voice: "F1"), system: FakeEngine(), model: model,
            output: output)

        do {
            try await speaker.speak("Hi", language: "en")
            XCTFail("It spoke")
        } catch {
            XCTAssertEqual(error, failure)
        }
        XCTAssertEqual(output.events.last, .stop)
    }

    func testAudioThatCantStartIsThrown() async {
        let output = FakeOutput(startError: .audioUnavailable(reason: "busy"))
        let speaker = speaker(installed: [], choice: .system, system: FakeEngine(), output: output)

        do {
            try await speaker.speak("Hi", language: "en")
            XCTFail("It spoke")
        } catch {
            XCTAssertEqual(error, .audioUnavailable(reason: "busy"))
        }
    }

    func testStopCutsOffWhatsPlaying() async throws {
        let output = FakeOutput()
        let system = FakeEngine(pieces: [[0.1]], holds: "A long text")
        let speaker = speaker(installed: [], choice: .system, system: system, output: output)

        let speaking = Task { try await speaker.speak("A long text", language: "en") }
        await system.started()
        await speaker.stop()
        try await speaking.value

        XCTAssertEqual(output.events.last, .stop)
        XCTAssertFalse(output.events.contains(.drain))
    }

    func testSpeakingAgainStopsTheOneBefore() async throws {
        let output = FakeOutput()
        let system = FakeEngine(pieces: [[0.1]], holds: "First")
        let speaker = speaker(installed: [], choice: .system, system: system, output: output)

        let first = Task { try await speaker.speak("First", language: "en") }
        await system.started()
        try await speaker.speak("Second", language: "en")
        try await first.value

        XCTAssertEqual(system.spoken.map(\.text), ["First", "Second"])
        let stop = try XCTUnwrap(output.events.firstIndex(of: .stop))
        let secondStart = try XCTUnwrap(output.events.lastIndex(of: .start(16_000, 1)))
        XCTAssertLessThan(stop, secondStart, "The first stopped before the second started")
        XCTAssertEqual(output.events.last, .drain)
    }
}

/// An engine that hands over `pieces`, then throws `error` if given. It
/// holds on after each piece of `holds` until it's cancelled.
final class FakeEngine: SpeechEngine, @unchecked Sendable {
    struct Spoken: Equatable {
        var text: String
        var language: String
        var voice: String
    }

    private let lock = NSLock()
    private var _spoken: [Spoken] = []
    private var waiters: [CheckedContinuation<Void, Never>] = []
    private var didStart = false
    let pieces: [[Float]]
    let error: SpeechError?
    let holds: String?

    init(pieces: [[Float]] = [[0]], error: SpeechError? = nil, holds: String? = nil) {
        self.pieces = pieces
        self.error = error
        self.holds = holds
    }

    var spoken: [Spoken] { lock.withLock { _spoken } }

    /// Returns once it's handed over its first piece.
    func started() async {
        await withCheckedContinuation { continuation in
            let ready = lock.withLock {
                if didStart { return true }
                waiters.append(continuation)
                return false
            }
            if ready { continuation.resume() }
        }
    }

    func synthesize(
        _ text: String, language: String, voice: String, audio: @escaping @Sendable (SpeechAudio) -> Void
    ) async throws(SpeechError) {
        lock.withLock { _spoken.append(Spoken(text: text, language: language, voice: voice)) }
        for piece in pieces {
            audio(SpeechAudio(samples: piece, sampleRate: 16_000))
            let waiting = lock.withLock {
                didStart = true
                let waiting = waiters
                waiters = []
                return waiting
            }
            for waiter in waiting { waiter.resume() }
            while text == holds, !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(1))
            }
        }
        if let error { throw error }
    }
}

/// Records what it's asked to play.
final class FakeOutput: AudioOutput, @unchecked Sendable {
    enum Event: Equatable {
        case start(Double, Float)
        case play([Float])
        case drain
        case stop
    }

    private let lock = NSLock()
    private var _events: [Event] = []
    private let startError: SpeechError?

    init(startError: SpeechError? = nil) {
        self.startError = startError
    }

    var events: [Event] { lock.withLock { _events } }

    func start(sampleRate: Double, speed: Float) async throws(SpeechError) {
        lock.withLock { _events.append(.start(sampleRate, speed)) }
        if let startError { throw startError }
    }

    func play(_ audio: SpeechAudio) async { lock.withLock { _events.append(.play(audio.samples)) } }
    func drain() async { lock.withLock { _events.append(.drain) } }
    func stop() async { lock.withLock { _events.append(.stop) } }
}
