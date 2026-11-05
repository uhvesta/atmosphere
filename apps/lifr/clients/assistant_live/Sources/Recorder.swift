import AVFAudio
import Foundation
import LifrAssistantClient

extension RecorderClient {
    /// Records from the device's microphone, as Gemma hears best: 16 kHz
    /// mono 16-bit WAV, for at most `RecorderClient.limit` seconds. The
    /// recording is in a temporary file only while it's made.
    package static func live() -> Self {
        let recorder = LiveRecorder()
        return RecorderClient(
            start: { try await recorder.start() },
            stop: { try await recorder.stop() },
            cancel: { await recorder.cancel() })
    }
}

/// One recording at a time.
private actor LiveRecorder {
    private var recorder: AVAudioRecorder?

    func start() async throws(AttachmentError) {
        guard await AVAudioApplication.requestRecordPermission() else { throw .microphoneDenied }
        cancel()
        let url = FileManager.default.temporaryDirectory.appending(path: "Recording-\(UUID().uuidString).wav")
        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatLinearPCM,
            AVSampleRateKey: 16_000,
            AVNumberOfChannelsKey: 1,
            AVLinearPCMBitDepthKey: 16,
            AVLinearPCMIsFloatKey: false,
            AVLinearPCMIsBigEndianKey: false,
        ]
        do {
            #if os(iOS)
                let session = AVAudioSession.sharedInstance()
                try session.setCategory(.record, mode: .spokenAudio)
                try session.setActive(true)
            #endif
            let recorder = try AVAudioRecorder(url: url, settings: settings)
            guard recorder.record(forDuration: TimeInterval(RecorderClient.limit)) else {
                throw AttachmentError.recordingFailed("the microphone didn't start")
            }
            self.recorder = recorder
        } catch let error as AttachmentError {
            throw error
        } catch {
            throw .recordingFailed(error.localizedDescription)
        }
    }

    func stop() throws(AttachmentError) -> RecorderClient.Recording {
        guard let recorder else { throw .recordingFailed("nothing was recording") }
        self.recorder = nil
        // Past the limit the recorder has stopped by itself, and its time
        // is back at zero.
        let seconds = recorder.isRecording ? recorder.currentTime : TimeInterval(RecorderClient.limit)
        recorder.stop()
        deactivate()
        defer { try? FileManager.default.removeItem(at: recorder.url) }
        do {
            let data = try Data(contentsOf: recorder.url)
            return RecorderClient.Recording(
                data: data, seconds: min(RecorderClient.limit, max(1, Int(seconds.rounded()))))
        } catch {
            throw .recordingFailed(error.localizedDescription)
        }
    }

    func cancel() {
        guard let recorder else { return }
        self.recorder = nil
        recorder.stop()
        recorder.deleteRecording()
        deactivate()
    }

    private func deactivate() {
        #if os(iOS)
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        #endif
    }
}
