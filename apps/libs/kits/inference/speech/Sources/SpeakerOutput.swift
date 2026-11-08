@preconcurrency import AVFoundation
import InferenceKit

/// The device's speaker, through AVAudioEngine: one player node, which
/// queues each piece of speech as it's made.
public actor SpeakerOutput: AudioOutput {
    private let engine = AVAudioEngine()
    private let player = AVAudioPlayerNode()
    private var format: AVAudioFormat?
    /// Pieces queued and not yet played.
    private var queued = 0
    /// Counts stops, so a piece one drops isn't counted as played later.
    private var generation = 0
    private var waiting: [CheckedContinuation<Void, Never>] = []

    public init() {
        engine.attach(player)
    }

    public func start(sampleRate: Double) throws(SpeechError) {
        #if os(iOS)
            do {
                let session = AVAudioSession.sharedInstance()
                try session.setCategory(.playback, mode: .spokenAudio, options: [.duckOthers])
                try session.setActive(true)
            } catch {
                throw .audioUnavailable(reason: error.localizedDescription)
            }
        #endif
        if format?.sampleRate != sampleRate {
            guard let format = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 1) else {
                throw .audioUnavailable(reason: "no audio format at \(Int(sampleRate)) Hz")
            }
            if engine.isRunning { engine.stop() }
            engine.disconnectNodeOutput(player)
            engine.connect(player, to: engine.mainMixerNode, format: format)
            self.format = format
        }
        if !engine.isRunning {
            engine.prepare()
            do {
                try engine.start()
            } catch {
                throw .audioUnavailable(reason: error.localizedDescription)
            }
        }
        if !player.isPlaying { player.play() }
    }

    public func play(_ audio: SpeechAudio) {
        guard let format, !audio.samples.isEmpty,
            let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(audio.samples.count)),
            let channel = buffer.floatChannelData
        else { return }
        buffer.frameLength = AVAudioFrameCount(audio.samples.count)
        audio.samples.withUnsafeBufferPointer { samples in
            guard let base = samples.baseAddress else { return }
            channel[0].update(from: base, count: samples.count)
        }
        queued += 1
        let generation = generation
        player.scheduleBuffer(buffer, completionCallbackType: .dataPlayedBack) { [weak self] _ in
            guard let self else { return }
            Task { await self.played(in: generation) }
        }
    }

    public func drain() async {
        guard queued > 0 else { return }
        await withCheckedContinuation { waiting.append($0) }
    }

    public func stop() {
        generation += 1
        player.stop()
        queued = 0
        wake()
        #if os(iOS)
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        #endif
    }

    private func played(in generation: Int) {
        guard generation == self.generation else { return }
        queued = max(queued - 1, 0)
        if queued == 0 { wake() }
    }

    private func wake() {
        let waiting = waiting
        self.waiting = []
        for continuation in waiting { continuation.resume() }
    }
}
