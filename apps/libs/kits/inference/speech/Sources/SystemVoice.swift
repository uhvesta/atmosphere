import AVFoundation
import InferenceKit

/// The system voice for a language, which the OS ships and needs no
/// download, as audio: AVSpeechSynthesizer writes it out a buffer at a
/// time rather than playing it, so it plays through the same player as a
/// voice model.
public struct SystemVoice: SpeechEngine {
    private let rate: Float

    /// - Parameter rate: How fast it speaks, against the system's default.
    public init(rate: Float = 1) {
        self.rate = rate
    }

    public func synthesize(
        _ text: String, language: String, voice: String, audio: @escaping @Sendable (SpeechAudio) -> Void
    ) async throws(SpeechError) {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw .nothingToSay }
        // With no voice for the language, the synthesizer reads the text in
        // the default voice, which can't read it, rather than failing.
        guard let voice = Self.voice(for: language) else { throw .systemVoiceFailed(language: language) }
        let writer = Writer()
        let wrote = await withTaskCancellationHandler {
            await withCheckedContinuation { (done: CheckedContinuation<Bool, Never>) in
                writer.write(text, voice: voice, rate: rate, audio: audio) { done.resume(returning: $0) }
            }
        } onCancel: {
            writer.stop()
        }
        guard wrote || Task.isCancelled else { throw .systemVoiceFailed(language: language) }
    }

    /// The system's voice for `language`, a BCP 47 tag: one for the tag
    /// itself, else any installed voice of its base language.
    static func voice(for language: String) -> AVSpeechSynthesisVoice? {
        if let voice = AVSpeechSynthesisVoice(language: language) { return voice }
        let base = VoiceModel.base(of: language)
        return AVSpeechSynthesisVoice.speechVoices().first { VoiceModel.base(of: $0.language) == base }
    }
}

/// One synthesizer, kept until it's written everything, which hands back
/// whether it made any audio once, at the end or when stopped.
private final class Writer: @unchecked Sendable {
    // Guarded by `lock`.
    private let lock = NSLock()
    private var synthesizer: AVSpeechSynthesizer?
    private var finish: ((Bool) -> Void)?
    private var made = false
    private var stopped = false

    func write(
        _ text: String, voice: AVSpeechSynthesisVoice, rate: Float,
        audio: @escaping @Sendable (SpeechAudio) -> Void, finish: @escaping (Bool) -> Void
    ) {
        let utterance = AVSpeechUtterance(string: text)
        utterance.voice = voice
        utterance.rate = min(
            max(AVSpeechUtteranceDefaultSpeechRate * rate, AVSpeechUtteranceMinimumSpeechRate),
            AVSpeechUtteranceMaximumSpeechRate)
        let synthesizer = AVSpeechSynthesizer()
        lock.withLock {
            self.synthesizer = synthesizer
            self.finish = finish
        }
        if lock.withLock({ stopped }) {
            end()
            return
        }
        synthesizer.write(utterance) { [self] buffer in
            guard let pcm = buffer as? AVAudioPCMBuffer, pcm.frameLength > 0 else {
                end()
                return
            }
            let samples = Self.samples(of: pcm)
            guard !samples.isEmpty else { return }
            lock.withLock { made = true }
            audio(SpeechAudio(samples: samples, sampleRate: pcm.format.sampleRate))
        }
    }

    func stop() {
        let synthesizer = lock.withLock {
            stopped = true
            return self.synthesizer
        }
        synthesizer?.stopSpeaking(at: .immediate)
        end()
    }

    private func end() {
        let (finish, made) = lock.withLock {
            let finish = self.finish
            self.finish = nil
            synthesizer = nil
            return (finish, self.made)
        }
        finish?(made)
    }

    /// The first channel's samples, as floats from -1 to 1.
    private static func samples(of buffer: AVAudioPCMBuffer) -> [Float] {
        let count = Int(buffer.frameLength)
        if let floats = buffer.floatChannelData {
            return Array(UnsafeBufferPointer(start: floats[0], count: count))
        }
        if let ints = buffer.int16ChannelData {
            return UnsafeBufferPointer(start: ints[0], count: count).map { Float($0) / Float(Int16.max) }
        }
        if let ints = buffer.int32ChannelData {
            return UnsafeBufferPointer(start: ints[0], count: count).map { Float($0) / Float(Int32.max) }
        }
        return []
    }
}
