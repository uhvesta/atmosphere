import AVFAudio
import Foundation
import Speech

/// Why dictation couldn't start or stopped early.
public enum DictationError: Error, Hashable, Sendable, CustomStringConvertible {
    /// The person hasn't let the app recognize speech.
    case speechDenied
    /// The person hasn't let the app use the microphone.
    case microphoneDenied
    /// This device can't recognize the language on the device itself, so
    /// nothing was sent anywhere. The language's name, as the person reads it.
    case unavailable(language: String)
    /// The microphone or the recognizer stopped, with the system's reason.
    case failed(String)

    public var description: String {
        let app = Self.appName
        switch self {
        case .speechDenied:
            #if os(macOS)
                return String(
                    localized: """
                        \(app) isn't allowed to recognize speech. Turn on \(app) in System Settings › Privacy & \
                        Security › Speech Recognition, then tap the microphone again.
                        """)
            #else
                return String(
                    localized: """
                        \(app) isn't allowed to recognize speech. Turn on Speech Recognition for \(app) in \
                        Settings › \(app), then tap the microphone again.
                        """)
            #endif
        case .microphoneDenied:
            #if os(macOS)
                return String(
                    localized: """
                        \(app) isn't allowed to use the microphone. Turn on \(app) in System Settings › Privacy & \
                        Security › Microphone, then tap the microphone again.
                        """)
            #else
                return String(
                    localized: """
                        \(app) isn't allowed to use the microphone. Turn on Microphone for \(app) in Settings › \
                        \(app), then tap the microphone again.
                        """)
            #endif
        case .unavailable(let language):
            #if os(macOS)
                return String(
                    localized: """
                        This Mac can't dictate \(language) on the device, and \(app) doesn't send your voice \
                        anywhere else. In System Settings › Keyboard, turn on Dictation with \(language) so it \
                        downloads, or type your message.
                        """)
            #else
                return String(
                    localized: """
                        This iPhone can't dictate \(language) on the device, and \(app) doesn't send your voice \
                        anywhere else. In Settings › General › Keyboard, turn on Dictation and add a \(language) \
                        keyboard so it downloads, or type your message.
                        """)
            #endif
        case .failed(let reason):
            return String(
                localized: """
                    Dictation stopped (\(reason)). Tap the microphone to try again; if another app is using the \
                    microphone, close it first.
                    """)
        }
    }

    private static var appName: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String
            ?? Bundle.main.object(forInfoDictionaryKey: "CFBundleName") as? String ?? "This app"
    }
}

/// Speech to text on the device only: what the person says, written out as
/// they speak, until stopped. Nothing they say leaves the device. One
/// dictation at a time.
public final class Dictation: @unchecked Sendable {
    private let lock = NSLock()
    private var engine: AVAudioEngine?
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var task: SFSpeechRecognitionTask?
    private var stopping = false

    public init() {}

    /// Asks for speech recognition and the microphone if the app hasn't
    /// yet, then listens. The stream has everything heard so far each time
    /// it grows, and ends once `stop` has the last of it; it throws a
    /// `DictationError`.
    public func start(locale: Locale = .current) async throws(DictationError) -> AsyncThrowingStream<String, any Error>
    {
        cancel()
        guard await Self.speechAllowed() else { throw .speechDenied }
        guard await AVAudioApplication.requestRecordPermission() else { throw .microphoneDenied }
        let language = locale.localizedString(forIdentifier: locale.identifier) ?? locale.identifier
        guard let recognizer = SFSpeechRecognizer(locale: locale), recognizer.supportsOnDeviceRecognition,
            recognizer.isAvailable
        else { throw .unavailable(language: language) }
        let request = SFSpeechAudioBufferRecognitionRequest()
        request.requiresOnDeviceRecognition = true
        request.shouldReportPartialResults = true
        request.addsPunctuation = true
        let engine = AVAudioEngine()
        do {
            #if os(iOS)
                let session = AVAudioSession.sharedInstance()
                try session.setCategory(.record, mode: .measurement, options: .duckOthers)
                try session.setActive(true, options: .notifyOthersOnDeactivation)
            #endif
            let input = engine.inputNode
            let format = input.outputFormat(forBus: 0)
            guard format.sampleRate > 0, format.channelCount > 0 else { throw DictationError.failed("no microphone") }
            input.installTap(onBus: 0, bufferSize: 1024, format: format) { buffer, _ in request.append(buffer) }
            engine.prepare()
            try engine.start()
        } catch let error as DictationError {
            Self.deactivate()
            throw error
        } catch {
            Self.deactivate()
            throw .failed(error.localizedDescription)
        }
        let (stream, continuation) = AsyncThrowingStream.makeStream(of: String.self)
        let task = recognizer.recognitionTask(with: request) { [weak self] result, error in
            if let result {
                continuation.yield(result.bestTranscription.formattedString)
                if result.isFinal { continuation.finish() }
            } else if let error {
                // Once stopped, an error only says nothing more was heard.
                if self?.isStopping == true {
                    continuation.finish()
                } else {
                    continuation.finish(throwing: DictationError.failed(error.localizedDescription))
                }
            }
        }
        continuation.onTermination = { [weak self] _ in self?.tearDown(cancelling: false) }
        lock.withLock {
            self.engine = engine
            self.request = request
            self.task = task
            stopping = false
        }
        return stream
    }

    /// Stops listening; the stream ends once the last words are written.
    public func stop() {
        let (engine, request) = lock.withLock { () -> (AVAudioEngine?, SFSpeechAudioBufferRecognitionRequest?) in
            stopping = true
            return (self.engine, self.request)
        }
        engine?.stop()
        engine?.inputNode.removeTap(onBus: 0)
        request?.endAudio()
        Self.deactivate()
    }

    /// Stops listening and throws away what's not yet written.
    public func cancel() {
        tearDown(cancelling: true)
    }

    private var isStopping: Bool { lock.withLock { stopping } }

    private func tearDown(cancelling: Bool) {
        let (engine, task) = lock.withLock { () -> (AVAudioEngine?, SFSpeechRecognitionTask?) in
            let taken = (self.engine, self.task)
            self.engine = nil
            request = nil
            self.task = nil
            stopping = cancelling
            return taken
        }
        guard let engine else { return }
        if engine.isRunning {
            engine.stop()
            engine.inputNode.removeTap(onBus: 0)
        }
        if cancelling { task?.cancel() }
        Self.deactivate()
    }

    private static func speechAllowed() async -> Bool {
        switch SFSpeechRecognizer.authorizationStatus() {
        case .authorized: return true
        case .denied, .restricted: return false
        case .notDetermined:
            return await withCheckedContinuation { continuation in
                SFSpeechRecognizer.requestAuthorization { continuation.resume(returning: $0 == .authorized) }
            }
        @unknown default: return false
        }
    }

    private static func deactivate() {
        #if os(iOS)
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        #endif
    }
}
